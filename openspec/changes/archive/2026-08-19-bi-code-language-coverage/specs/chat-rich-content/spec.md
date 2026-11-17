# chat-rich-content — Spec Delta

## ADDED Requirements

### Requirement: A fence tag resolves to a grammar, a name, and nothing else

The system MUST resolve a fence's language tag through one registry that
answers three questions: which highlight grammar renders it, what the language
is called, and whether the tag is recognized at all. No other place may decide
any of the three, so a language cannot be highlighted under one name and
labelled under another.

A tag naming a dialect of a language MUST highlight with that language's
grammar while keeping its own display name. The dialect tag is the most
specific thing the model said about the code, and losing either the
highlighting or the precision is a worse answer than keeping both.

An unrecognized tag MUST render as plain text and MUST be shown verbatim as
the artifact's name. Inventing a display name for a tag the registry does not
know would tell the reader something untrue about their code.

A fence with no tag MUST render as plain text under a generic name, and MUST
NOT be auto-detected. A misdetected grammar highlights the wrong tokens, which
reads as authoritative and is harder to disbelieve than no highlighting.

#### Scenario: A dialect-tagged query

- **GIVEN** a fence tagged `sparksql`
- **THEN** it highlights with the SQL grammar and the card names it
  `Spark SQL`

#### Scenario: A tag the registry does not know

- **GIVEN** a fence tagged `zzql`
- **THEN** the code renders unhighlighted and the card names it `zzql`

#### Scenario: An untagged fence

- **GIVEN** a fence with no language tag whose body is not a recognized
  document
- **THEN** the code renders unhighlighted under a generic name, with no
  grammar guessed for it

### Requirement: The grammar set covers the languages a data platform answers in

The system MUST register grammars for the languages its own answers arrive in,
rather than inheriting a general-purpose default set. The set MUST include SQL
and its dialects, Python, R, Scala, Julia, SAS, Stata, MATLAB, YAML, JSON,
TOML/INI, XML, Markdown, shell and PowerShell, Dockerfile, GraphQL, and
diff/patch.

The system MUST register grammars for DAX, KQL, and Power Query M. These are
first-class query languages of the platforms this product connects to and
highlight.js ships no grammar for any of them, so absent a grammar written
here they cannot be rendered as anything but grey text.

Registering a grammar MUST NOT depend on the model tagging a fence the way the
grammar is named — that is what the alias half of the registry is for.

#### Scenario: A DAX measure

- **GIVEN** a fence tagged `dax` holding a `CALCULATE`/`FILTER` measure
- **THEN** its keywords, functions, strings, and comments are highlighted, and
  the card names it `DAX`

#### Scenario: A dbt model configuration

- **GIVEN** a fence tagged `yml`
- **THEN** it highlights as YAML and the card names it `YAML`

### Requirement: Tab-separated data renders as a table

The system MUST render a `tsv` fence as a data table, through the same parser
that renders `csv`, with the delimiter as that parser's parameter. A warehouse
CLI export is at least as likely to be tab-separated as comma-separated, and
two parsers for one shape would drift on quoting, escapes, and ragged rows —
the three things the parser exists to get right.

A structural parse failure MUST fall through to the code card, as it does for
`csv`, so malformed data renders as its own text rather than as nothing.

#### Scenario: A tab-separated export

- **GIVEN** a fence tagged `tsv` with a header row and three data rows
- **THEN** it renders as a table of three rows, with the same row and column
  count in the card's description as `csv` would report

#### Scenario: A quoted field containing a tab

- **GIVEN** a `tsv` fence whose quoted field contains a tab character
- **THEN** the field is one cell, because the shared parser's quoting rules
  apply to both delimiters
