## Validation

- [x] 1. Unit tests prove text, CSV and workbook reading, per-file and total bounds, line-boundary cuts, type mismatch, unsupported names, and the image allowance.
- [x] 2. Mock-DA tests prove each turn gets its own files through both presigned and on-prem storage, the presigned GET carries no bearer, an unfetchable file is named, and an unreadable list fails the run.
- [x] 3. Adapter tests prove the OpenAI-compatible, Anthropic and Google request shapes carry a turn's image.
- [x] 4. Renderer tests prove Attach opens the picker first, a cancel leaves nothing, a send opens the reserved conversation, and leaving releases it.
- [x] 5. A title test proves a files-only message is titled by its file names.
- [ ] 6. A signed-in person attaches a CSV on the new-chat screen, asks about it, and gets an answer that uses it.
