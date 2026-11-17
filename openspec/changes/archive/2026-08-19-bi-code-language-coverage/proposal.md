# Every language a BI answer arrives in renders as that language

## Why

The fence ladder in `AssistantMessage.tsx` gives five content types a body of
their own — HTML documents, SVG, markdown, JSON arrays, CSV — and hands
everything else to the code card, which highlights through `rehype-highlight`
on its default grammar set. That default is lowlight's `common`: 37 grammars,
out of the 192 highlight.js ships.

`common` contains `sql`. It contains nothing else a data platform's answers are
written in, and the `sql` grammar carries no aliases. Probed against the
installed dependency:

```
sql         registered  tokens=8
sparksql    NOT REGISTERED -> renders as plain text
tsql        NOT REGISTERED -> renders as plain text
plsql       NOT REGISTERED -> renders as plain text
postgres    NOT REGISTERED -> renders as plain text
pgsql       NOT REGISTERED -> renders as plain text
snowflake   NOT REGISTERED -> renders as plain text
bigquery    NOT REGISTERED -> renders as plain text
databricks  NOT REGISTERED -> renders as plain text
hive        NOT REGISTERED -> renders as plain text
dax         NOT REGISTERED -> renders as plain text
kql         NOT REGISTERED -> renders as plain text
m           NOT REGISTERED -> renders as plain text
tsv         NOT REGISTERED -> renders as plain text
```

So the product's own subject matter is its worst-rendered content. A model
asked about a Snowflake warehouse writes ` ```snowflake `, a Databricks
one writes ` ```sparksql `, a Fabric semantic model produces ```` ```dax

`````— and every one of those lands as undifferentiated grey monospace, while a
```` ```kotlin ```` fence nobody will ever send highlights correctly. The
dialect tag is not noise the model should be talked out of emitting; it is the
single most useful thing it can say about a query, and it is exactly what turns
highlighting off today.

Three consequences:

1. **A dialect tag is a penalty.** ```` ```sql ```` highlights and
   ```` ```snowflake ```` does not, so the more precisely the model names what
   it wrote, the worse the reader's copy looks.
2. **The BI-native query languages have no grammar at all.** DAX, KQL, and
   Power Query M are not in highlight.js under any name. These are not exotic —
   they are what Power BI, Fabric, Azure Monitor, and Excel's own query engine
   speak.
3. **The card names the tag, not the language.** The header reads `py`, `tsx`,
   `yml`, `sparksql` — the literal fence token, whatever casing the model chose.
   A reader gets the model's shorthand instead of the language's name.

Separately, `csv` has a table rung and `tsv` does not, though a tab-separated
export is the more common shape coming out of a warehouse CLI. The parser
already handles quoting, escapes, and ragged rows; only the delimiter is
hard-coded.

## What Changes

- **One language registry** resolves a fence tag to three things: the grammar
  that highlights it, the name shown on the card, and whether it is a known
  language at all. Aliases are declared once, so a SQL dialect highlights as
  SQL and is still *called* T-SQL, Snowflake SQL, or Spark SQL on the card —
  the reader keeps the precision the model offered, and gets highlighting too.
- **The grammar set is chosen for this product** rather than inherited:
  `common`, plus the highlight.js grammars a data platform needs and `common`
  omits (PostgreSQL, Scala, Julia, PowerShell, Dockerfile, SAS, Stata, MATLAB),
  plus three grammars written here because highlight.js has none — **DAX**,
  **KQL**, and **Power Query M**.
- **Twelve SQL dialect tags** resolve to the SQL grammar: `tsql`, `plsql`,
  `mysql`, `postgres`/`postgresql`/`pgsql`, `snowflake`, `bigquery`,
  `redshift`, `databricks`/`sparksql`, `hive`/`hiveql`, `trino`/`presto`,
  `duckdb`, `athena`.
- **A `tsv` fence renders as a table**, through the same parser as `csv` with
  the delimiter as a parameter.
- **The card header names the language.** `SQL`, `Snowflake SQL`, `Python`,
  `DAX`, `Power Query M` — never `py` or `sparksql`. An unknown tag is still
  shown verbatim, because inventing a name for a tag we do not recognize is
  worse than repeating it.

## What does not change

- **No new preview rung, and no mermaid.** Rendering a diagram means running a
  layout engine over untrusted text and inserting the `<style>` and `<svg>` it
  produces. The app CSP is `script-src 'self'; style-src 'self'` — no
  `unsafe-inline`, no `unsafe-eval` — and `richContent`'s standing rule is that
  nothing here injects markup into the app document. A diagram rung therefore
  needs either a CSP relaxation or an asset route on the `xf-preview` scheme so
  the renderer can run inside the sandbox that already exists for HTML
  documents. That is a `preview-protocol` change with its own security
  argument, not a line in a language table, and it is deliberately left out of
  this one.
- **No second highlighter and no worker.** xfalcon-web offloads highlighting to
  `markdown-worker.ts` because it renders in a browser tab competing with
  hydration. This runtime has no such contention and the frontend privilege
  boundary keeps the renderer thin; a worker here would be latency spent on a
  problem the desktop does not have.
- **The sanitize-then-highlight order stays.** Untrusted model output is
  sanitized first; the schema's `language-*` allowance on `<code>` is what
  carries the fence tag through to the highlighter, and it already admits every
  tag added here.
`````
