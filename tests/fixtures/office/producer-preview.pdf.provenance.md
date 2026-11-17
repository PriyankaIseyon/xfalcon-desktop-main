# PDF preview fixture

`producer-preview.pdf` is a static two-page PDF kept to exercise the PDF viewer
with a real file (`tests/visual/office-pdf.spec.ts`). It was made once from this
repository's synthetic `src-tauri/tests/fixtures/office/producer.pptx` with
LibreOffice 26.8.1.1. The app does not produce it: previews are made at run time
by an installed Microsoft Office program, and nothing here depends on LibreOffice.

- Size: 41,890 bytes
- SHA-256: `5c421a43d64fadc8f0211f0f0c2c8b0c3b81aaf5d5cf33386dd4e25cecb847c1`
- The original PPTX fixture is not modified by this test.
