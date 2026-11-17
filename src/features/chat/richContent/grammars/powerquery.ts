import type { HLJSApi, Language } from "highlight.js";

/**
 * Power Query M — the transformation language behind Power BI's and Excel's
 * query editors, and the language a Dataflow is written in. highlight.js has no
 * grammar for it, so an `m` or `powerquery` fence renders as grey text without
 * this.
 *
 * Data, not logic — see the note in `./dax.ts`. Case-SENSITIVE: M's keywords are
 * lowercase and its library functions are `Table.SelectRows` exactly, so
 * folding case would make a genuine identifier error look correct.
 *
 * The standard library is matched by SHAPE rather than enumerated. It runs to
 * roughly a thousand functions across `Table.*`, `List.*`, `Text.*`,
 * `Record.*`, and a connector namespace per data source; a literal list would
 * be stale the week it was written and would still miss the connector the user
 * happens to be querying.
 */
export function powerquery(hljs: HLJSApi): Language {
  return {
    name: "Power Query M",
    aliases: ["powerquery", "pq"],
    keywords: {
      keyword: [
        "let",
        "in",
        "if",
        "then",
        "else",
        "each",
        "try",
        "otherwise",
        "error",
        "as",
        "is",
        "meta",
        "type",
        "section",
        "shared",
        "and",
        "or",
        "not",
        "optional",
        "nullable",
      ],
      literal: ["true", "false", "null"],
      type: [
        "any",
        "anynonnull",
        "binary",
        "date",
        "datetime",
        "datetimezone",
        "duration",
        "function",
        "list",
        "logical",
        "none",
        "number",
        "record",
        "table",
        "text",
        "time",
      ],
    },
    contains: [
      hljs.COMMENT("//", "$"),
      hljs.C_BLOCK_COMMENT_MODE,
      // A doubled quote is an escaped quote. `#(lf)` and friends are M's
      // character escapes and stay inside the string.
      { className: "string", begin: '"', end: '"', contains: [{ begin: '""' }, { begin: /#\([^)]*\)/ }] },
      // #"Quoted identifier" — a step or column name with spaces in it. This is
      // an identifier, not a string, and a reader of a generated query relies
      // on telling "Changed Type" the step from "Changed Type" the value.
      { className: "symbol", begin: '#"', end: '"', illegal: /\n/ },
      // Intrinsic literals: #date(2026, 8, 12), #duration(0, 1, 0, 0), #table.
      {
        className: "meta",
        begin: /#(?:date|datetime|datetimezone|duration|time|binary|table|sections|shared|infinity|nan)\b/,
      },
      // `Table.SelectRows` — namespace-dot-function, the shape of every library
      // call. Matched as a shape so a connector we have never heard of still
      // reads as a library call rather than as two identifiers.
      { className: "built_in", begin: /\b[A-Z][A-Za-z0-9]*\.[A-Za-z][A-Za-z0-9]*/ },
      hljs.C_NUMBER_MODE,
      { className: "operator", begin: /=>|<>|>=|<=|\.\.|[=<>+\-*/&?]/ },
    ],
  };
}
