# Tasks

Registry first, then the grammars it names, then the two consumers. The
registry is the contract both the highlighter and the card header read, so it
settles before either of them changes.

## 1. The three missing grammars

- [x] 1.1 `richContent/grammars/dax.ts` — a `LanguageFn` for DAX: line
      comments (`//`), block comments, double-quoted strings with `""`
      escapes, single-quoted table references, `[Column]` references, numbers,
      and the keyword/function split (`VAR`/`RETURN`/`EVALUATE` as keywords,
      `CALCULATE`/`FILTER`/`SUMMARIZE` and the time-intelligence family as
      built-ins).
- [x] 1.2 `richContent/grammars/kql.ts` — Kusto: `//` comments, strings,
      timespan literals (`7d`, `30m`), the tabular operators (`where`,
      `summarize`, `project`, `extend`, `join`, `render`) and the aggregation
      functions, with `|` recognized as the pipe it is.
- [x] 1.3 `richContent/grammars/powerquery.ts` — Power Query M: `//` and
      `/* */`, `"` strings with `""` escapes, `#"quoted identifiers"`,
      `#date`/`#datetime`/`#duration` literals, `let`/`in`/`each`/`try`/
      `otherwise` keywords, and the `Table.*`/`List.*`/`Text.*` library shape.
- [x] 1.4 Each grammar is data, not logic: no `eval`, no regex built from
      model input, `contains` only. A grammar is fed untrusted text on every
      keystroke of a stream.

## 2. The registry

- [x] 2.1 `richContent/codeLanguages.ts` exports one table keyed by canonical
      grammar id, each entry carrying its display name, its grammar (or none,
      for tags that resolve to another entry's grammar), and its alias tags
      with their own display names where the dialect deserves one.
- [x] 2.2 `resolveLanguage(tag)` returns `{ grammar, displayName }` or the
      unknown answer. One function; the highlighter and the card header both
      call it, so a language cannot highlight as one thing and be labelled
      another.
- [x] 2.3 `HIGHLIGHT_LANGUAGES` (name → grammar) and `HIGHLIGHT_ALIASES`
      (grammar name → alias tags) are derived FROM the table, not maintained
      beside it — a grammar registered without its aliases, or an alias with no
      grammar, must be impossible rather than merely discouraged.
- [x] 2.4 The twelve SQL dialects, and the display name each one keeps.
- [x] 2.5 `detect: false` stays. An untagged fence gets no grammar guessed.

## 3. Delimited data

- [x] 3.1 `parseCsv(code)` becomes `parseDelimited(code, delimiter)`. One
      parser, delimiter as a parameter — the quoting, `""` escape, CRLF, and
      ragged-row rules are what it exists for and must not be written twice.
      Update the call site and the tests; no compatibility wrapper.
- [x] 3.2 A `tsv` fence takes the table rung, with the same building
      placeholder and the same fall-through to the code card on a structural
      parse failure.

## 4. The consumers

- [x] 4.1 `AssistantMessage.tsx` passes the registry's grammars and aliases to
      `rehypeHighlight`, in both the top-level pipeline and the nested-markdown
      one, so a fence inside a markdown fence highlights identically.
- [x] 4.2 `CodeArtifact` takes the resolved display name rather than the raw
      tag, and `Copy` still yields the fence body byte-for-byte.
- [x] 4.3 The sanitize schema is unchanged and re-verified: `language-*` on
      `<code>` already admits every tag added here, and sanitize still runs
      before highlight.

## 5. Tests

- [x] 5.1 Registry: every alias resolves to a registered grammar, every
      grammar in the table is registered, and no tag appears twice. Table-driven
      over the real table, so a future entry is checked by existing tests.
- [x] 5.2 One test per named language proving it highlights: SQL and each of
      the twelve dialects, DAX, KQL, M, Python, R, YAML, JSON, TOML, diff. The
      assertion is that tokens were produced AND the card names the language.
- [x] 5.2a Exhaustive rather than per-named-language: a sample per canonical
      entry, driven off `LANGUAGE_TABLE`, so adding a language without proving
      it highlights fails the suite. Every alias is checked for BOTH a grammar
      and a name — an alias that highlights but resolves to no name is the exact
      failure this registry exists to prevent.
- [x] 5.3 An unknown tag renders unhighlighted under its own verbatim name; an
      untagged fence renders unhighlighted under a generic name.
- [x] 5.4 `tsv` renders as a table, including a quoted field holding a tab; a
      malformed `tsv` falls through to the code card.
- [x] 5.5 The three new grammars are inert against hostile input: a fence body
      carrying `<script>`, `javascript:`, and an HTML comment highlights as
      text and reaches the DOM as text.

## 6. Verification

- [x] 6.1 `npx tsc --noEmit`, `npm run lint`, `npx vitest run`.
- [x] 6.2 Bundle delta measured and stated. Grammars are data; a set this size
      should cost tens of kilobytes, and a surprise means something pulled in
      more than a grammar.
- [x] 6.3 Live: a fence per named language in one answer, read on screen.

## Deviations from this plan, as built

- **The dialect pattern went past SQL.** Task 2.4 named only the twelve SQL
  dialects, but highlight.js has three other grammars that serve two languages
  under one name: `xml` is "HTML, XML" and `ini` is "TOML, also INI". A `svg`
  fence holding a FRAGMENT falls through to the code card, and labelling it
  "XML" there answers a question the reader did not ask — so `html`, `svg`, and
  `toml` each get their own entry pointing at the shared grammar, exactly as
  `snowflake` points at `sql`. `shell` stayed its own entry rather than becoming
  an alias of `bash`: it is a separate grammar in `common` (a session with a
  prompt, not a script), and aliasing a registered grammar name would have
  shadowed it.
- **The hostile-input test asserts tree shape, not the absence of a string.**
  The first draft asserted `onerror` did not appear anywhere in the highlighted
  tree, which is the opposite of the property wanted: "classified as text" means
  the bytes must still BE there, as a text node, because Copy has to yield the
  fence body byte-for-byte. The test now walks the tree and proves every element
  the grammar produced is a `span` carrying nothing but `className`, and
  separately that the concatenated text is the input unchanged.
- **Bundle delta measured (task 6.2): +63.7 kB raw, +25.2 kB gzip** — 822.5 kB
  to 886.1 kB. Eight highlight.js grammars and three written here; grammars are
  data, and the number is in the range the task predicted, so nothing pulled in
  more than a grammar.
- **Coverage is enforced, not asserted (task 5.2a).** The first pass tested the
  languages the proposal happened to name — about 42 of the 135 tags. It now
  walks `LANGUAGE_TABLE` itself: 55 canonical entries each with a realistic
  sample carrying a comment, a string, or a number (a keyword list alone can pass
  while a grammar's comment and string modes are wrong), and all 135 tags
  checked for both a registered grammar and a display name. `plaintext` is the
  one entry asserted to produce ZERO tokens — it is plain text by definition, and
  a grammar finding something to highlight in it would be wrong.
