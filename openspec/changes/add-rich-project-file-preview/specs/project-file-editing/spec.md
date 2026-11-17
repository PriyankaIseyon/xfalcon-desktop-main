## ADDED Requirements

### Requirement: A file renders the way the chat renders its type

The system MUST render a project file by its type, using the same artifact cards the
chat transcript uses for that type. The type SHALL come from the file's extension, or
from a well-known file name such as `Dockerfile` or `Makefile`, compared without regard
to case:

- HTML (`.html`, `.htm`, `.xhtml`) as the live document preview;
- SVG (`.svg`) as the image preview;
- Markdown (`.md`, `.markdown`) as prose, through the chat's markdown pipeline;
- CSV (`.csv`) and TSV (`.tsv`, `.tab`) as a data table, through the chat's one
  delimited-data parser;
- JSON (`.json`) as a data table when it is a non-empty array of objects, and as
  highlighted JSON otherwise; JSON Lines (`.jsonl`, `.ndjson`) as a data table when
  every non-blank line is an object;
- every language the chat's registry names, as the code card under that language's
  name, highlighted with the same grammar.

A file of any other type, and a file whose content fails its type's parser, MUST render
as plain text. Malformed content MUST NOT render blank.

#### Scenario: An HTML dashboard

- **GIVEN** a project file `reports/Q3.html` holding a complete document
- **WHEN** it is opened in the file view
- **THEN** it renders as the live preview, in the same frame the chat uses

#### Scenario: A CSV file

- **GIVEN** `data/regions.csv` with a header row and three data rows
- **THEN** it renders as a table of three rows, with the chat's row cap and panel offer

#### Scenario: A SQL file

- **GIVEN** `queries/pipeline.snowflake.sql`
- **THEN** it renders as a highlighted code card named SQL

#### Scenario: A malformed CSV

- **GIVEN** a `.csv` file with an unterminated quote
- **THEN** it renders as plain text

#### Scenario: An unknown type

- **GIVEN** a file with an extension no rendering claims
- **THEN** it renders as plain text, as before

### Requirement: Rendering a file adds no trust

The system MUST render HTML and SVG files only in the sandboxed frame loaded from the
preview origin (`preview-protocol`), with scripts but without same-origin access, exactly
as the chat does. Markdown MUST pass the chat's sanitizer. Table cells and code MUST
render as text nodes.

The file's bytes MUST NOT reach the app's own document as markup by any other route.

#### Scenario: A hostile HTML file

- **GIVEN** a project HTML file whose script tries to read app storage or call the app
- **WHEN** it is previewed
- **THEN** it cannot, because the frame has no same-origin access and no network

#### Scenario: Markdown with raw HTML

- **GIVEN** a Markdown file containing a `<script>` element
- **THEN** the element is removed by the sanitizer and nothing runs

### Requirement: A rich view needs the whole file

The system MUST render a document, table, or Markdown file richly only from the whole
file. A file too large to hold whole (`add-large-local-project-io`) SHALL be shown a
window at a time as plain text, or, for a delimited file, as backend-paged rows.

Each rich rendering MUST have its own size bound, measured in the file's bytes, and a
file past it MUST render as plain lines with a note saying why, rather than stall the
view: highlighting past 2 MiB, Markdown prose past 4 MiB, an in-webview table past
16 MiB, a preview past 32 MiB. Plain lines MUST be virtualized, so only the lines on
screen exist in the DOM, whatever the file's length.

#### Scenario: A large HTML file

- **GIVEN** an HTML file larger than the preview bound but small enough to hold whole
- **THEN** it shows as plain numbered lines with a note that it is too large to
  preview, and it is not registered for preview

#### Scenario: A large code file

- **GIVEN** a 3 MiB SQL file
- **THEN** it shows as plain numbered lines with a note that it is too large to
  highlight

### Requirement: Editing works on the source

The system MUST keep editing on the file's text, whatever its rendering. Edit SHALL open
the text editor over the exact bytes read, and leaving edit mode SHALL return to the
file's rendering, showing the saved text.

#### Scenario: Editing a Markdown file

- **GIVEN** a rendered Markdown file
- **WHEN** the person edits and saves it
- **THEN** the editor showed the raw text, and after the save the view renders the
  saved text again
