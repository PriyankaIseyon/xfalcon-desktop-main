# Office reader fixtures

`producer.docx` was generated with `python-docx` 1.2.0. It contains Unicode
text split across runs, XML entities, a table, a referenced header and footer,
and a tracked deletion added to the package XML. The deletion checks that
revision text is excluded.

`producer.pptx` was generated with `python-pptx` 1.0.2. It contains two slides,
a table, speaker notes, and a chart with cached category and value text. The
slide part files were renamed to `zeta.xml` and `alpha.xml`; the presentation
relationships and content types were updated. This checks that extraction
follows presentation order and relationship targets instead of sorting part
names.

These files contain synthetic test content and were made for this repository's
parser tests. They do not include images with text, macros, or external links.

`no-text.docx` and `no-text.pptx` are minimal synthetic OOXML packages authored
for the attachment and search tests. They contain an empty Word paragraph and
an empty PowerPoint slide, respectively. `image-only.docx` and
`image-only.pptx` contain one embedded synthetic 1x1 PNG and no text. These
packages use only minimal OOXML parts and synthetic content. `legacy-poi/empty.doc`
is the Apache POI fixture documented in `legacy-poi/PROVENANCE.txt`. The no-text
legacy PPT used by tests is generated from the bounded synthetic persist-graph
builder in `documents/legacy.rs`; its current slide is empty while the package
also has unreferenced historical slide text to verify current-graph selection.
