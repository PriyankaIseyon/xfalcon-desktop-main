import { common } from "lowlight";
import type { LanguageFn } from "highlight.js";
import dockerfile from "highlight.js/lib/languages/dockerfile";
import julia from "highlight.js/lib/languages/julia";
import matlab from "highlight.js/lib/languages/matlab";
import pgsql from "highlight.js/lib/languages/pgsql";
import powershell from "highlight.js/lib/languages/powershell";
import sas from "highlight.js/lib/languages/sas";
import scala from "highlight.js/lib/languages/scala";
import stata from "highlight.js/lib/languages/stata";
import { dax } from "./grammars/dax";
import { kql } from "./grammars/kql";
import { powerquery } from "./grammars/powerquery";

/**
 * Which languages the transcript renders, what each one is called, and which
 * grammar highlights it — in one table, because those three answers have to
 * agree (spec: chat-rich-content).
 *
 * **The problem this exists to fix.** `rehype-highlight` defaults to lowlight's
 * `common`: 37 grammars out of highlight.js's 192. `common` carries `sql`, the
 * `sql` grammar declares no aliases, and nothing else a data platform answers in
 * is present. So `sql` highlighted and `sparksql`, `tsql`, `plsql`, `postgres`,
 * `snowflake`, `bigquery`, `databricks`, `hive`, `dax`, `kql`, and `m` all
 * rendered as plain grey text — the more precisely the model named the dialect
 * it wrote, the worse the reader's copy looked.
 *
 * **Dialects keep their name.** A `snowflake` fence highlights with the SQL
 * grammar and is still called "Snowflake SQL" on the card. The dialect tag is
 * the most specific thing the model said about that query; throwing away either
 * the highlighting or the precision is a worse answer than keeping both.
 *
 * **No detection.** `detect: false` stays set at the call site. A guessed
 * grammar highlights the wrong tokens with total confidence, which is harder for
 * a reader to disbelieve than no highlighting at all.
 */

type LanguageEntry = {
  /** What the card calls it. */
  name: string;
  /** The grammar to register under this id. Absent when the id is only ever an
   * alias of another entry — no id may carry both. */
  grammar?: LanguageFn;
  /**
   * The grammar this id highlights WITH, when that is a different entry. A
   * dialect points at its parent language here and keeps its own `name`.
   */
  highlightAs?: string;
  /** Other tags a model may write for this same language, each keeping this
   * entry's name. A tag needing its own name gets its own entry instead. */
  aliases?: string[];
};

/** Grammars `common` already registers. Listed so the display name is ours
 * rather than highlight.js's internal one, and so a `common` grammar and an
 * added one are declared the same way. */
const COMMON_GRAMMAR = undefined;

const LANGUAGES: Record<string, LanguageEntry> = {
  // ── SQL and its dialects ────────────────────────────────────────────────
  // Every dialect highlights with the one `sql` grammar. Writing a grammar per
  // dialect would mean maintaining twelve keyword lists to render the same
  // SELECT twelve identical ways.
  sql: { name: "SQL", grammar: COMMON_GRAMMAR, aliases: ["ansi", "sqlite"] },
  tsql: { name: "T-SQL", highlightAs: "sql", aliases: ["mssql", "sqlserver", "synapse", "fabricsql"] },
  plsql: { name: "PL/SQL", highlightAs: "sql", aliases: ["oracle", "oraclesql"] },
  mysql: { name: "MySQL", highlightAs: "sql", aliases: ["mariadb"] },
  snowflake: { name: "Snowflake SQL", highlightAs: "sql", aliases: ["snowsql"] },
  bigquery: { name: "BigQuery SQL", highlightAs: "sql", aliases: ["googlesql", "standardsql"] },
  redshift: { name: "Redshift SQL", highlightAs: "sql" },
  sparksql: { name: "Spark SQL", highlightAs: "sql", aliases: ["databricks", "databrickssql"] },
  hiveql: { name: "HiveQL", highlightAs: "sql", aliases: ["hive"] },
  trino: { name: "Trino SQL", highlightAs: "sql", aliases: ["presto", "prestosql"] },
  duckdb: { name: "DuckDB SQL", highlightAs: "sql", aliases: ["duckdbsql"] },
  athena: { name: "Athena SQL", highlightAs: "sql" },
  // PostgreSQL has its OWN grammar in highlight.js — the only dialect that
  // does — and it is not in `common`. Its aliases come from the grammar itself,
  // and are re-declared here so this table is the whole answer.
  pgsql: { name: "PostgreSQL", grammar: pgsql, aliases: ["postgres", "postgresql"] },

  // ── The BI query languages highlight.js does not ship ───────────────────
  dax: { name: "DAX", grammar: dax },
  kql: { name: "KQL", grammar: kql, aliases: ["kusto"] },
  powerquery: { name: "Power Query M", grammar: powerquery, aliases: ["m", "pq"] },

  // ── Analysis and modelling ─────────────────────────────────────────────
  python: { name: "Python", grammar: COMMON_GRAMMAR, aliases: ["py", "gyp", "ipython"] },
  r: { name: "R", grammar: COMMON_GRAMMAR, aliases: ["rscript"] },
  scala: { name: "Scala", grammar: scala },
  julia: { name: "Julia", grammar: julia },
  sas: { name: "SAS", grammar: sas },
  stata: { name: "Stata", grammar: stata, aliases: ["do"] },
  matlab: { name: "MATLAB", grammar: matlab },

  // ── Configuration, the shape a warehouse project is defined in ─────────
  yaml: { name: "YAML", grammar: COMMON_GRAMMAR, aliases: ["yml", "lookml", "dbt"] },
  json: { name: "JSON", grammar: COMMON_GRAMMAR, aliases: ["jsonc", "jsonl", "ndjson"] },
  // One grammar, two names — highlight.js calls it "TOML, also INI". A `toml`
  // fence is not an INI file and saying so on the card would be wrong, so the
  // dialect gets its own entry the way a SQL dialect does.
  ini: { name: "INI", grammar: COMMON_GRAMMAR, aliases: ["conf", "properties", "cfg"] },
  toml: { name: "TOML", highlightAs: "ini" },
  // One grammar ("HTML, XML" in highlight.js), three names. A fence tagged
  // `svg` that holds a FRAGMENT rather than a standalone document falls through
  // to the code card, and calling it XML there would answer a question the
  // reader did not ask.
  xml: { name: "XML", grammar: COMMON_GRAMMAR, aliases: ["xsd", "xsl", "xslt", "rss", "atom", "plist"] },
  html: { name: "HTML", highlightAs: "xml", aliases: ["htm", "xhtml"] },
  svg: { name: "SVG", highlightAs: "xml" },
  graphql: { name: "GraphQL", grammar: COMMON_GRAMMAR, aliases: ["gql"] },
  dockerfile: { name: "Dockerfile", grammar: dockerfile, aliases: ["docker"] },

  // ── Shells and glue ────────────────────────────────────────────────────
  // `shell` is its own grammar in `common` — a transcript with a prompt, not a
  // script — so it keeps its own entry rather than becoming an alias of bash.
  bash: { name: "Bash", grammar: COMMON_GRAMMAR, aliases: ["sh", "zsh", "ksh"] },
  shell: { name: "Shell session", grammar: COMMON_GRAMMAR, aliases: ["console"] },
  powershell: { name: "PowerShell", grammar: powershell, aliases: ["pwsh", "ps1"] },
  makefile: { name: "Makefile", grammar: COMMON_GRAMMAR, aliases: ["make", "mk"] },
  diff: { name: "Diff", grammar: COMMON_GRAMMAR, aliases: ["patch", "udiff"] },

  // ── Application languages, for the code an answer quotes ───────────────
  typescript: { name: "TypeScript", grammar: COMMON_GRAMMAR, aliases: ["ts", "tsx", "mts", "cts"] },
  javascript: { name: "JavaScript", grammar: COMMON_GRAMMAR, aliases: ["js", "jsx", "mjs", "cjs"] },
  rust: { name: "Rust", grammar: COMMON_GRAMMAR, aliases: ["rs"] },
  go: { name: "Go", grammar: COMMON_GRAMMAR, aliases: ["golang"] },
  java: { name: "Java", grammar: COMMON_GRAMMAR },
  kotlin: { name: "Kotlin", grammar: COMMON_GRAMMAR, aliases: ["kt"] },
  csharp: { name: "C#", grammar: COMMON_GRAMMAR, aliases: ["cs", "dotnet"] },
  cpp: { name: "C++", grammar: COMMON_GRAMMAR, aliases: ["cc", "hpp"] },
  c: { name: "C", grammar: COMMON_GRAMMAR, aliases: ["h"] },
  ruby: { name: "Ruby", grammar: COMMON_GRAMMAR, aliases: ["rb"] },
  php: { name: "PHP", grammar: COMMON_GRAMMAR },
  swift: { name: "Swift", grammar: COMMON_GRAMMAR },
  lua: { name: "Lua", grammar: COMMON_GRAMMAR },
  css: { name: "CSS", grammar: COMMON_GRAMMAR },
  scss: { name: "SCSS", grammar: COMMON_GRAMMAR, aliases: ["sass"] },
  less: { name: "Less", grammar: COMMON_GRAMMAR },
  markdown: { name: "Markdown", grammar: COMMON_GRAMMAR, aliases: ["md", "mkdown"] },
  plaintext: { name: "Plain text", grammar: COMMON_GRAMMAR, aliases: ["text", "txt", "log"] },
};

/** Table-driven, so the two derived maps below and the display lookup can never
 * disagree about which tag means which language. Every alias resolves here to
 * the canonical id it belongs to. */
const CANONICAL_BY_TAG: Map<string, string> = (() => {
  const map = new Map<string, string>();
  for (const [id, entry] of Object.entries(LANGUAGES)) {
    map.set(id, id);
    for (const alias of entry.aliases ?? []) map.set(alias, id);
  }
  return map;
})();

/**
 * Grammars to register with the highlighter: `common`, plus every grammar this
 * table adds. Derived from the table rather than maintained beside it — a
 * grammar registered without its aliases, or an alias pointing at nothing, has
 * to be impossible rather than merely discouraged.
 */
export const HIGHLIGHT_LANGUAGES: Record<string, LanguageFn> = (() => {
  const languages: Record<string, LanguageFn> = { ...common };
  for (const [id, entry] of Object.entries(LANGUAGES)) {
    if (entry.grammar !== undefined) languages[id] = entry.grammar;
  }
  return languages;
})();

/**
 * Tag → registered grammar name, for every tag that is not itself a registered
 * grammar: an alias of a language, or a dialect highlighting as its parent.
 * This is what turns a `snowflake` fence into highlighted SQL.
 */
export const HIGHLIGHT_ALIASES: Record<string, string[]> = (() => {
  const aliases: Record<string, string[]> = {};
  const add = (grammar: string, tag: string) => {
    (aliases[grammar] ??= []).push(tag);
  };
  for (const [id, entry] of Object.entries(LANGUAGES)) {
    const grammar = entry.highlightAs ?? id;
    // A dialect's own id is an alias too: `tsql` is not a registered grammar.
    if (entry.highlightAs !== undefined) add(grammar, id);
    for (const alias of entry.aliases ?? []) add(grammar, alias);
  }
  return aliases;
})();

export type ResolvedLanguage = {
  /** What the card calls it. An unrecognized tag is its own name, verbatim. */
  name: string;
  /** Whether the registry knows this tag — so a caller can tell "no tag" and
   * "a tag we do not render" apart from a language. */
  known: boolean;
};

/** A fence with no tag. Named generically because there is nothing truthful to
 * call it, and never auto-detected. */
const UNTAGGED: ResolvedLanguage = { name: "Code", known: false };

/**
 * What to call the code in a fence tagged `tag`.
 *
 * An unknown tag is returned verbatim rather than replaced with "Code": the
 * model wrote `zzql` for a reason the reader may recognize even when we do not,
 * and inventing a name would state something untrue about their code.
 */
export function resolveLanguage(tag: string): ResolvedLanguage {
  const normalized = tag.trim().toLowerCase();
  if (normalized === "") return UNTAGGED;
  const canonical = CANONICAL_BY_TAG.get(normalized);
  if (canonical === undefined) return { name: tag.trim(), known: false };
  // The dialect's own entry, not its parent's — a `snowflake` fence highlights
  // as SQL and is still called Snowflake SQL.
  return { name: LANGUAGES[canonical]!.name, known: true };
}

/** Every tag the registry answers for, canonical ids and aliases alike. Exists
 * for the tests that walk the whole table. */
export function knownLanguageTags(): string[] {
  return [...CANONICAL_BY_TAG.keys()];
}

/** The table itself, for the test that checks each entry declares exactly one
 * of `grammar` / `highlightAs` and that every `highlightAs` names a real
 * registered grammar. */
export const LANGUAGE_TABLE: Readonly<Record<string, LanguageEntry>> = LANGUAGES;

/**
 * The language a fence's `language-*` class names, lowercased, or "" when the
 * class is absent. Shared by the highlighter's plugin and the component that
 * picks a card, so the two cannot read a fence's language differently.
 */
export function languageOf(className: string | undefined): string {
  const match = /language-([\w:+-]+)/.exec(className ?? "");
  return (match?.[1] ?? "").toLowerCase();
}
