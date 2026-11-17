use crate::preview::protocol::{inject_base_style, respond};
use crate::preview::store::{PreviewDocumentStore, PreviewKind};

fn header<'a>(
    response: &'a http::Response<std::borrow::Cow<'static, [u8]>>,
    name: &str,
) -> &'a str {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
}

#[test]
fn html_document_serves_with_document_policy() {
    let store = PreviewDocumentStore::default();
    let id = store
        .register(
            PreviewKind::Html,
            "<!doctype html><html><head></head><body>r</body></html>",
        )
        .expect("register");
    let response = respond(&store, &format!("/{id}"));
    assert_eq!(response.status(), http::StatusCode::OK);
    assert_eq!(
        header(&response, "Content-Type"),
        "text/html; charset=utf-8"
    );
    assert_eq!(header(&response, "X-Content-Type-Options"), "nosniff");
    let csp = header(&response, "Content-Security-Policy").to_owned();
    assert!(csp.contains("default-src 'none'"), "network denied: {csp}");
    assert!(csp.contains("connect-src 'none'"), "exfil denied: {csp}");
    assert!(
        csp.contains("style-src 'unsafe-inline'"),
        "authored styles allowed: {csp}"
    );
    assert!(
        csp.contains("script-src 'unsafe-inline'"),
        "authored scripts allowed: {csp}"
    );
}

/// `'self'` is not a usable font source for these documents. The frame is
/// sandboxed without `allow-same-origin`, so the document runs on an opaque
/// origin that is same-origin with nothing — `'self'` would match no URL,
/// including the `/fonts/` routes this handler serves, and an unstyled
/// document would fall back to the browser serif the base sheet exists to
/// prevent.
#[test]
fn document_policies_name_the_font_origin_rather_than_self() {
    let store = PreviewDocumentStore::default();
    let html = store
        .register(PreviewKind::Html, "<!doctype html><p>r")
        .expect("register html");
    let svg = store
        .register(
            PreviewKind::Svg,
            "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        )
        .expect("register svg");

    for id in [html, svg] {
        let response = respond(&store, &format!("/{id}"));
        let csp = header(&response, "Content-Security-Policy").to_owned();
        assert!(
            csp.contains("font-src xf-preview://localhost http://xf-preview.localhost data:"),
            "both platform forms of the font origin are named: {csp}"
        );
        assert!(
            !csp.contains("'self'"),
            "an opaque origin makes 'self' match nothing: {csp}"
        );
    }
}

/// The Windows spelling of the preview origin is written out in policy
/// strings, which cannot branch on the platform. It must stay the scheme's
/// own host.
#[test]
fn preview_http_host_matches_the_scheme() {
    assert_eq!(
        crate::preview::PREVIEW_HTTP_HOST,
        format!("{}.localhost", crate::preview::PREVIEW_URI_SCHEME)
    );
}

#[test]
fn svg_document_serves_as_image_with_scripts_refused() {
    let store = PreviewDocumentStore::default();
    let id = store
        .register(
            PreviewKind::Svg,
            "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        )
        .expect("register");
    let response = respond(&store, &format!("/{id}"));
    assert_eq!(
        header(&response, "Content-Type"),
        "image/svg+xml; charset=utf-8"
    );
    assert!(header(&response, "Content-Security-Policy").contains("script-src 'none'"));
    // Served verbatim — no base-style injection into svg documents.
    assert!(!String::from_utf8_lossy(response.body()).contains("data-xf-preview-base"));
}

#[test]
fn unknown_id_answers_404() {
    let store = PreviewDocumentStore::default();
    let response = respond(&store, "/deadbeef");
    assert_eq!(response.status(), http::StatusCode::NOT_FOUND);
}

#[test]
fn percent_encoded_paths_route_like_plain_ones() {
    let store = PreviewDocumentStore::default();
    let id = store
        .register(PreviewKind::Html, "<html>e</html>")
        .expect("register");
    let encoded: String = id.chars().map(|c| format!("%{:02x}", c as u32)).collect();
    let response = respond(&store, &format!("/{encoded}"));
    assert_eq!(response.status(), http::StatusCode::OK);
}

#[test]
fn bundled_fonts_are_served_for_the_base_stylesheet() {
    let store = PreviewDocumentStore::default();
    // (file, content-type, magic): Google Sans = original TTFs (sfnt magic
    // 00 01 00 00), Geist Mono = woff2 (wOF2).
    for (file, content_type, magic) in [
        (
            "GoogleSans-Variable.ttf",
            "font/ttf",
            &[0x00u8, 0x01, 0x00, 0x00][..],
        ),
        (
            "GoogleSans-Italic.ttf",
            "font/ttf",
            &[0x00u8, 0x01, 0x00, 0x00][..],
        ),
        ("GeistMono.woff2", "font/woff2", b"wOF2" as &[u8]),
    ] {
        let response = respond(&store, &format!("/fonts/{file}"));
        assert_eq!(response.status(), http::StatusCode::OK, "{file}");
        assert_eq!(header(&response, "Content-Type"), content_type, "{file}");
        // Sandboxed preview documents fetch fonts from an opaque origin
        // (`Origin: null`); CORS must admit them.
        assert_eq!(
            header(&response, "Access-Control-Allow-Origin"),
            "*",
            "{file}"
        );
        assert_eq!(&response.body()[..4], magic, "{file}");
    }
    let missing = respond(&store, "/fonts/Nope.woff2");
    assert_eq!(missing.status(), http::StatusCode::NOT_FOUND);
}

#[test]
fn base_style_lands_inside_head_when_present() {
    let out = inject_base_style("<!doctype html><html><head><title>t</title></head><body/></html>");
    let head_open = out.find("<head>").expect("head");
    let style = out.find("<style data-xf-preview-base>").expect("style");
    let title = out.find("<title>").expect("title");
    assert!(
        head_open < style && style < title,
        "injected first inside head"
    );
}

#[test]
fn base_style_falls_back_to_html_then_prepend() {
    let after_html = inject_base_style("<html lang=\"en\"><body>x</body></html>");
    assert!(after_html.starts_with("<html lang=\"en\"><style data-xf-preview-base>"));

    let prepended = inject_base_style("<p>fragmentish</p>");
    assert!(prepended.starts_with("<style data-xf-preview-base>"));
}

#[test]
fn header_lookalike_tags_do_not_swallow_the_injection() {
    // `<header>` must not be mistaken for `<head>`.
    let out = inject_base_style("<html><body><header>h</header></body></html>");
    assert!(out.starts_with("<html><style data-xf-preview-base>"));
}

#[test]
fn authored_rules_beat_injected_defaults_by_construction() {
    // The injected block is zero-specificity `:where()` rules only — assert
    // the invariant that keeps authored documents authoritative.
    let out = inject_base_style("<html><head></head></html>");
    let style_start = out.find("<style data-xf-preview-base>").expect("style");
    let style_end = out.find("</style>").expect("close");
    let css = &out[style_start..style_end];
    for selector_start in css.match_indices('{').map(|(i, _)| i) {
        let rule_head = &css[..selector_start];
        let last_rule = rule_head.rsplit('}').next().unwrap_or(rule_head);
        assert!(
            last_rule.contains(":where(") || last_rule.contains("@font-face"),
            "non-zero-specificity rule found: {last_rule}"
        );
    }
}
