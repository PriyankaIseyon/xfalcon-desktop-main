//! The `xf-preview` URI scheme handler.
//!
//! URL forms are platform-split by wry: `http://xf-preview.localhost/<path>`
//! on Windows/Android, `xf-preview://localhost/<path>` elsewhere. Routes:
//!
//! - `/<doc-id>` — a registered document, served with its kind's
//!   Content-Type and the per-document CSP (D3);
//! - `/fonts/<file>` — the bundled product faces, so the injected base
//!   stylesheet (D5) can reach them under the document CSP's `font-src`.
//!
//! Responses are content-addressed, so caching is `immutable`; an evicted
//! id simply 404s on the next cold load.

use std::borrow::Cow;
use std::sync::Arc;

use tauri::{Manager, Runtime, UriSchemeContext};

use super::store::{PreviewDocumentStore, PreviewKind};

pub const PREVIEW_URI_SCHEME: &str = "xf-preview";

/// The same origin's Windows form. wry maps a custom scheme to
/// `http://<scheme>.localhost` under WebView2 and to `<scheme>://localhost`
/// everywhere else, and the frontend's `convertFileSrc` picks the form from
/// the user agent. A policy string cannot branch on the platform, so every
/// policy that spells this origin out names both forms.
/// `preview_http_host_matches_the_scheme` pins the two together.
pub const PREVIEW_HTTP_HOST: &str = "xf-preview.localhost";

/// The preview origin as a CSP source list, in both platform forms.
///
/// `'self'` cannot stand in for it. Every preview frame is sandboxed WITHOUT
/// `allow-same-origin`, so the document runs on an opaque origin that is
/// same-origin with nothing — `'self'` then matches no URL at all, including
/// the font routes this very handler serves. Naming the origin is also no
/// wider than `'self'` was meant to be: the scheme has exactly one handler,
/// and it answers with in-memory documents and three bundled faces.
///
/// A macro, not a `const`, because the policies below are `concat!`ed at
/// compile time — a `const` cannot be spliced into a string literal, and the
/// two policies would otherwise each carry a hand-copied duplicate.
macro_rules! preview_origin_sources {
    () => {
        "xf-preview://localhost http://xf-preview.localhost"
    };
}

/// The app's own origins, in the packaged and dev-server forms of both
/// platforms, for `frame-ancestors`.
macro_rules! app_origin_sources {
    () => {
        "http://tauri.localhost tauri://localhost http://127.0.0.1:1420 http://localhost:1420"
    };
}

/// Per-document policy (spec: preview-protocol D3). Inline style/script are
/// the point — previewed documents are self-contained by product contract —
/// while every network avenue stays dead: no host sources anywhere,
/// `connect-src 'none'`. Ancestors are pinned to the app origins (prod +
/// dev-server forms).
const HTML_DOCUMENT_CSP: &str = concat!(
    "default-src 'none'; style-src 'unsafe-inline'; ",
    "script-src 'unsafe-inline'; img-src data: blob:; font-src ",
    preview_origin_sources!(),
    " data:; connect-src 'none'; base-uri 'none'; form-action 'none'; ",
    "frame-ancestors ",
    app_origin_sources!(),
);

/// SVG previews carry image semantics: same network denial, and scripts are
/// refused outright (`script-src 'none'`).
const SVG_DOCUMENT_CSP: &str = concat!(
    "default-src 'none'; style-src 'unsafe-inline'; ",
    "script-src 'none'; img-src data: blob:; font-src ",
    preview_origin_sources!(),
    " data:; connect-src 'none'; base-uri 'none'; form-action 'none'; ",
    "frame-ancestors ",
    app_origin_sources!(),
);

/// Zero-specificity typography defaults (spec: preview-protocol D5). Every
/// rule is wrapped in `:where()` (specificity 0,0,0), so ANY authored rule —
/// even a bare element selector — wins over these. A document that says
/// nothing about type renders in the product faces on a light canvas
/// instead of the browser serif default.
const BASE_STYLE: &str = concat!(
    "<style data-xf-preview-base>",
    "@font-face{font-family:\"Google Sans\";src:url(\"/fonts/GoogleSans-Variable.ttf\")format(\"truetype\");font-weight:400 700;font-style:normal;font-display:swap}",
    "@font-face{font-family:\"Google Sans\";src:url(\"/fonts/GoogleSans-Italic.ttf\")format(\"truetype\");font-weight:400 700;font-style:italic;font-display:swap}",
    "@font-face{font-family:\"Geist Mono\";src:url(\"/fonts/GeistMono.woff2\")format(\"woff2\");font-weight:100 900;font-style:normal;font-display:swap}",
    ":where(html){background:#fff;color:#0b1220;font-family:\"Google Sans\";font-optical-sizing:auto;-webkit-font-smoothing:antialiased;line-height:1.55}",
    ":where(pre,code,kbd,samp){font-family:\"Geist Mono\"}",
    ":where(*){scrollbar-width:thin;scrollbar-color:#a0a6b0 transparent}",
    "</style>"
);

/// The faces ship inside the binary (compile-time copies of
/// `public/fonts/`, the same files the app itself loads) so the route works
/// identically under the dev server and the embedded `custom-protocol`
/// build. Google Sans is the ORIGINAL Google-shipped variable TTF pair,
/// unmodified.
const FONT_ROUTES: &[(&str, &str, &[u8])] = &[
    (
        "GoogleSans-Variable.ttf",
        "font/ttf",
        include_bytes!("../../../public/fonts/GoogleSans-Variable.ttf"),
    ),
    (
        "GoogleSans-Italic.ttf",
        "font/ttf",
        include_bytes!("../../../public/fonts/GoogleSans-Italic.ttf"),
    ),
    (
        "GeistMono.woff2",
        "font/woff2",
        include_bytes!("../../../public/fonts/GeistMono.woff2"),
    ),
];

pub fn handle_preview_request<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
) -> http::Response<Cow<'static, [u8]>> {
    let store = ctx.app_handle().state::<Arc<PreviewDocumentStore>>();
    respond(&store, request.uri().path())
}

/// Pure request→response core, separated from the Tauri context so tests
/// exercise the real routing, headers, and injection without an app handle.
pub(super) fn respond(
    store: &PreviewDocumentStore,
    raw_path: &str,
) -> http::Response<Cow<'static, [u8]>> {
    // `convertFileSrc` percent-encodes the path it is given; normalize
    // before routing so `doc%2Fid`-style forms and plain paths match alike.
    let path = percent_decode(raw_path);
    let path = path.trim_start_matches('/');

    if let Some(file) = path.strip_prefix("fonts/") {
        if let Some((_, content_type, bytes)) =
            FONT_ROUTES.iter().find(|(name, _, _)| *name == file)
        {
            return http::Response::builder()
                .status(http::StatusCode::OK)
                .header("Content-Type", *content_type)
                .header("X-Content-Type-Options", "nosniff")
                .header("Cache-Control", "public, max-age=31536000, immutable")
                // Font fetches are CORS-mode, and the sandboxed preview
                // document has an OPAQUE origin (`Origin: null`) — without
                // this header the base stylesheet's faces are blocked. The
                // fonts are public product assets; `*` grants read access to
                // exactly these bytes and nothing else.
                .header("Access-Control-Allow-Origin", "*")
                .body(Cow::Borrowed(*bytes))
                .expect("static preview font response");
        }
        return not_found();
    }

    let Some((kind, content)) = store.serve(path) else {
        return not_found();
    };
    let body: Vec<u8> = match kind {
        PreviewKind::Html => inject_base_style(&content).into_bytes(),
        PreviewKind::Svg => content.into_bytes(),
    };
    let csp = match kind {
        PreviewKind::Html => HTML_DOCUMENT_CSP,
        PreviewKind::Svg => SVG_DOCUMENT_CSP,
    };
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header("Content-Type", kind.content_type())
        .header("Content-Security-Policy", csp)
        .header("X-Content-Type-Options", "nosniff")
        .header("Cache-Control", "public, max-age=31536000, immutable")
        .body(Cow::Owned(body))
        .expect("static preview document response")
}

fn not_found() -> http::Response<Cow<'static, [u8]>> {
    http::Response::builder()
        .status(http::StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("X-Content-Type-Options", "nosniff")
        .body(Cow::Borrowed(b"preview document not found" as &[u8]))
        .expect("static preview 404 response")
}

/// Display-time only (the stored message bytes are never modified): insert
/// the base `<style>` where the parser will hoist it into `<head>` —
/// after the opening `<head…>`, else after `<html…>`, else prepended.
pub(super) fn inject_base_style(document: &str) -> String {
    let insert_at = tag_end("<head", document)
        .or_else(|| tag_end("<html", document))
        .unwrap_or(0);
    let mut out = String::with_capacity(document.len() + BASE_STYLE.len());
    out.push_str(&document[..insert_at]);
    out.push_str(BASE_STYLE);
    out.push_str(&document[insert_at..]);
    out
}

/// Byte offset just past the `>` of the first `tag` occurrence (ASCII
/// case-insensitive), when that occurrence really opens a tag.
fn tag_end(tag: &str, document: &str) -> Option<usize> {
    let lower = document.to_ascii_lowercase();
    let mut search_from = 0;
    while let Some(relative) = lower[search_from..].find(tag) {
        let start = search_from + relative;
        let after = lower.as_bytes().get(start + tag.len());
        let opens_tag = matches!(
            after,
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r')
        );
        if opens_tag {
            if let Some(close) = document[start..].find('>') {
                return Some(start + close + 1);
            }
        }
        search_from = start + tag.len();
    }
    None
}

/// Minimal %XX decoder — invalid escapes pass through verbatim, which can
/// only produce a non-existent id (404), never a different valid one.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &input[index + 1..index + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
