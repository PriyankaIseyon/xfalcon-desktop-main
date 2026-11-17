use crate::preview::store::{
    PreviewDocumentStore, PreviewKind, PreviewStoreError, PREVIEW_DOC_MAX_BYTES,
    PREVIEW_STORE_MAX_BYTES, RENDERER_PREVIEW_MAX_BYTES,
};

#[test]
fn register_then_serve_roundtrips_content_and_kind() {
    let store = PreviewDocumentStore::default();
    let id = store
        .register(
            PreviewKind::Html,
            "<!doctype html><html><body>hi</body></html>",
        )
        .expect("register");
    let (kind, content) = store.serve(&id).expect("serve");
    assert_eq!(kind, PreviewKind::Html);
    assert!(content.contains("hi"));
}

#[test]
fn identical_content_registers_to_the_same_id_once() {
    let store = PreviewDocumentStore::default();
    let first = store
        .register(PreviewKind::Html, "<html>same</html>")
        .expect("first");
    let second = store
        .register(PreviewKind::Html, "<html>same</html>")
        .expect("second");
    assert_eq!(first, second);
    assert_eq!(store.resident_bytes(), "<html>same</html>".len());
}

#[test]
fn kind_participates_in_the_address() {
    let store = PreviewDocumentStore::default();
    let html = store.register(PreviewKind::Html, "<svg/>").expect("html");
    let svg = store.register(PreviewKind::Svg, "<svg/>").expect("svg");
    assert_ne!(html, svg);
}

#[test]
fn ids_are_stable_across_store_instances() {
    let a = PreviewDocumentStore::default()
        .register(PreviewKind::Svg, "<svg viewBox=\"0 0 1 1\"/>")
        .expect("a");
    let b = PreviewDocumentStore::default()
        .register(PreviewKind::Svg, "<svg viewBox=\"0 0 1 1\"/>")
        .expect("b");
    assert_eq!(a, b);
}

#[test]
fn oversized_document_is_refused() {
    let store = PreviewDocumentStore::default();
    let oversized = "x".repeat(PREVIEW_DOC_MAX_BYTES + 1);
    assert_eq!(
        store.register(PreviewKind::Html, &oversized),
        Err(PreviewStoreError::DocumentTooLarge {
            limit: PREVIEW_DOC_MAX_BYTES
        })
    );
}

#[test]
fn unknown_id_serves_nothing() {
    let store = PreviewDocumentStore::default();
    assert!(store.serve("0".repeat(64).as_str()).is_none());
}

#[test]
fn eviction_drops_least_recently_served_documents_first() {
    // Tiny caps, same code path: two 40-byte documents fit a 100-byte store;
    // the third insert must evict exactly the stalest one.
    let store = PreviewDocumentStore::with_caps(64, 100);
    let first = store
        .register(PreviewKind::Html, "a".repeat(40))
        .expect("first");
    let second = store
        .register(PreviewKind::Html, "b".repeat(40))
        .expect("second");
    // Freshen `first` so `second` becomes the eviction candidate.
    assert!(store.serve(&first).is_some());
    let third = store
        .register(PreviewKind::Html, "c".repeat(40))
        .expect("third");
    assert!(store.serve(&first).is_some(), "freshened document survived");
    assert!(store.serve(&second).is_none(), "stale document evicted");
    assert!(store.serve(&third).is_some(), "new document resident");
    assert!(store.resident_bytes() <= 100);
}

#[test]
fn production_caps_hold_the_documented_values() {
    // A project file previews up to 32 MiB from disk; a chat card's document,
    // which the renderer supplies, stays at 2 MiB.
    assert_eq!(PREVIEW_DOC_MAX_BYTES, 32 * 1024 * 1024);
    assert_eq!(RENDERER_PREVIEW_MAX_BYTES, 2 * 1024 * 1024);
    assert_eq!(PREVIEW_STORE_MAX_BYTES, 256 * 1024 * 1024);
}
