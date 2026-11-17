import { createLowlight } from "lowlight";
import { describe, expect, it } from "vitest";
import {
  HIGHLIGHT_ALIASES,
  HIGHLIGHT_LANGUAGES,
  LANGUAGE_TABLE,
  knownLanguageTags,
  resolveLanguage,
} from "./codeLanguages";

/**
 * The registry, exercised through the SAME highlighter the transcript builds —
 * `createLowlight(HIGHLIGHT_LANGUAGES)` then `registerAlias(HIGHLIGHT_ALIASES)`
 * is exactly what `rehype-highlight` does with these two exports. A test that
 * only read the table would prove the table, not the rendering.
 */
function highlighter() {
  const lowlight = createLowlight(HIGHLIGHT_LANGUAGES);
  lowlight.registerAlias(HIGHLIGHT_ALIASES);
  return lowlight;
}

/** How many highlight spans a tag produces for a sample. Zero means the fence
 * rendered as undifferentiated grey text, which is the defect this change
 * exists to fix. */
function tokenCount(tag: string, code: string): number {
  const tree = highlighter().highlight(tag, code);
  return JSON.stringify(tree).split('"hljs-').length - 1;
}

const SELECT = "select count(*) as n from sales where region = 'EMEA' -- note";

describe("the language table", () => {
  it("declares exactly one of grammar or highlightAs for every entry", () => {
    for (const [id, entry] of Object.entries(LANGUAGE_TABLE)) {
      const both = entry.grammar !== undefined && entry.highlightAs !== undefined;
      expect(both, `${id} declares both a grammar and a highlightAs`).toBe(false);
    }
  });

  it("resolves every tag it answers for to a registered grammar", () => {
    const lowlight = highlighter();
    for (const tag of knownLanguageTags()) {
      expect(lowlight.registered(tag), `${tag} is not registered`).toBe(true);
    }
  });

  it("points every dialect at a grammar that exists", () => {
    const lowlight = highlighter();
    for (const [id, entry] of Object.entries(LANGUAGE_TABLE)) {
      if (entry.highlightAs === undefined) continue;
      expect(lowlight.registered(entry.highlightAs), `${id} -> ${entry.highlightAs}`).toBe(true);
    }
  });

  it("claims no tag twice", () => {
    const tags = knownLanguageTags();
    expect(new Set(tags).size).toBe(tags.length);
  });
});

describe("SQL dialects", () => {
  // The whole point: the more precisely the model named the dialect, the worse
  // the reader's copy used to look. Every one of these rendered as plain text
  // before this table existed.
  const DIALECTS: Array<[string, string]> = [
    ["sql", "SQL"],
    ["tsql", "T-SQL"],
    ["mssql", "T-SQL"],
    ["plsql", "PL/SQL"],
    ["oracle", "PL/SQL"],
    ["mysql", "MySQL"],
    ["snowflake", "Snowflake SQL"],
    ["bigquery", "BigQuery SQL"],
    ["redshift", "Redshift SQL"],
    ["sparksql", "Spark SQL"],
    ["databricks", "Spark SQL"],
    ["hive", "HiveQL"],
    ["trino", "Trino SQL"],
    ["presto", "Trino SQL"],
    ["duckdb", "DuckDB SQL"],
    ["athena", "Athena SQL"],
    ["postgres", "PostgreSQL"],
  ];

  it.each(DIALECTS)("highlights a %s query and calls it %s", (tag, name) => {
    expect(tokenCount(tag, SELECT)).toBeGreaterThan(0);
    expect(resolveLanguage(tag).name).toBe(name);
  });

  it("keeps the dialect's name rather than collapsing it to SQL", () => {
    // Highlighting with the parent grammar must not cost the reader the one
    // thing the tag told them.
    expect(resolveLanguage("snowflake").name).not.toBe("SQL");
    expect(resolveLanguage("sparksql").name).not.toBe("SQL");
  });
});

describe("the BI query languages highlight.js does not ship", () => {
  it("highlights a DAX measure", () => {
    const dax = [
      "// Rolling 12-month net sales",
      "Net Sales R12 =",
      "VAR Window = DATESINPERIOD ( 'Date'[Date], MAX ( 'Date'[Date] ), -12, MONTH )",
      "RETURN CALCULATE ( SUM ( Sales[NetAmount] ), Window, ALLEXCEPT ( Sales, Sales[Region] ) )",
    ].join("\n");
    expect(tokenCount("dax", dax)).toBeGreaterThan(4);
    expect(resolveLanguage("dax").name).toBe("DAX");
  });

  it("tells a DAX table reference from a string", () => {
    // 'Date' is a table, "Date" is a value. DAX spells them differently and a
    // reader relies on seeing which one they wrote.
    const tree = highlighter().highlight("dax", "EVALUATE FILTER ( 'Date', 'Date'[Year] = \"2026\" )");
    const rendered = JSON.stringify(tree);
    expect(rendered).toContain("hljs-symbol");
    expect(rendered).toContain("hljs-string");
    expect(rendered).toContain("hljs-attr");
  });

  it("highlights a KQL query and its timespan literals", () => {
    const kql = [
      "// Failed sign-ins by app",
      "SigninLogs",
      "| where TimeGenerated > ago(7d) and ResultType != 0",
      "| summarize failures = count() by AppDisplayName, bin(TimeGenerated, 30m)",
      "| order by failures desc",
    ].join("\n");
    expect(tokenCount("kql", kql)).toBeGreaterThan(4);
    expect(resolveLanguage("kusto").name).toBe("KQL");
  });

  it("highlights Power Query M and its quoted step names", () => {
    const m = [
      "let",
      '    Source = Sql.Database("warehouse", "analytics"),',
      '    #"Filtered Rows" = Table.SelectRows ( Source, each [Region] <> null ),',
      '    #"Changed Type" = Table.TransformColumnTypes ( #"Filtered Rows", {{"Amount", type number}} )',
      "in",
      '    #"Changed Type"',
    ].join("\n");
    expect(tokenCount("m", m)).toBeGreaterThan(4);
    expect(resolveLanguage("powerquery").name).toBe("Power Query M");
    expect(resolveLanguage("m").name).toBe("Power Query M");
  });

  it("reads a library call by shape, so an unknown connector still highlights", () => {
    // The M standard library runs to ~1000 functions across a namespace per
    // data source. Matching the shape is what keeps a connector nobody listed
    // from reading as two bare identifiers.
    const tree = highlighter().highlight("m", 'let x = Snowflake.Databases("acct", "WH") in x');
    expect(JSON.stringify(tree)).toContain("hljs-built_in");
  });
});

/**
 * One realistic sample per CANONICAL entry, driven off the registry itself.
 *
 * The point is exhaustiveness: the first assertion is that every entry in
 * `LANGUAGE_TABLE` has a sample here, so adding a language without proving it
 * highlights fails this suite rather than quietly shipping a tag that renders
 * grey. Samples carry a comment, a string, or a number on purpose — a keyword
 * list alone can pass while the grammar's comment and string modes are wrong.
 */
const SAMPLES: Record<string, string> = {
  // SQL and its dialects — all twelve share the one grammar, so they share the
  // one sample. The dialect-specific assertions are in the block above.
  sql: SELECT,
  tsql: SELECT,
  plsql: SELECT,
  mysql: SELECT,
  snowflake: SELECT,
  bigquery: SELECT,
  redshift: SELECT,
  sparksql: SELECT,
  hiveql: SELECT,
  trino: SELECT,
  duckdb: SELECT,
  athena: SELECT,
  pgsql: "SELECT net FROM sales WHERE region = 'EMEA'; -- note",

  // Written here, because highlight.js ships none of the three.
  dax: "// R12\nNet R12 = CALCULATE ( SUM ( Sales[Amount] ), ALLEXCEPT ( Sales, Sales[Region] ) )",
  kql: "// failures\nSigninLogs | where TimeGenerated > ago(7d) | summarize n = count() by AppDisplayName",
  powerquery: 'let Source = Sql.Database("wh", "analytics") in Source // load',

  // Analysis and modelling
  python: "def net(x):\n    return x * 1.2  # uplift",
  r: 'sales <- read.csv("q3.csv")  # load',
  scala: 'val df = spark.read.parquet("/mnt/sales") // load',
  julia: "function net(x)\n    return x * 1.2\nend",
  sas: "proc means data=sales; var amount; run; /* summary */",
  stata: "regress sales price // ols",
  matlab: "y = mean(sales(:, 2)); % column mean",

  // Configuration — the shape a warehouse project is defined in
  yaml: "models:\n  - name: fct_sales\n    description: 'grain: order'",
  json: '{"region": "EMEA", "net": 1200, "ok": true}',
  ini: '[profile]\ntarget = "prod"',
  toml: '[profile]\ntarget = "prod"',
  xml: '<Cube name="Sales"><Measure name="Net"/></Cube>',
  html: '<div class="kpi">Net</div>',
  svg: '<svg viewBox="0 0 4 4"><circle r="2"/></svg>',
  graphql: 'query Sales { sales(region: "EMEA") { net } }',
  dockerfile: "FROM python:3.12\nRUN pip install dbt-core",

  // Shells and glue
  bash: 'snow sql -q "select 1" # smoke',
  shell: '$ snow sql -q "select 1"',
  powershell: "Get-Content .\\query.sql | Invoke-Sqlcmd  # run",
  makefile: "build:\n\tdbt run --target prod",
  diff: "--- a/model.sql\n+++ b/model.sql\n-select 1\n+select 2",

  // Application languages, for the code an answer quotes
  typescript: "const net: number = 1200; // total",
  javascript: "const net = 1200; // total",
  rust: "let net: f64 = 1200.0; // total",
  go: "net := 1200 // total",
  java: "int net = 1200; // total",
  kotlin: "val net = 1200 // total",
  csharp: "var net = 1200; // total",
  cpp: "#include <cstdio>\nint net = 1200;",
  c: "#include <stdio.h>\nint net = 1200;",
  ruby: "net = 1200 # total",
  php: "<?php $net = 1200; // total",
  swift: "let net = 1200 // total",
  lua: "local net = 1200 -- total",
  css: ".kpi { color: #2ED1ED; }",
  scss: "$accent: #2ED1ED;\n.kpi { color: $accent; }",
  less: "@accent: #2ED1ED;\n.kpi { color: @accent; }",
  markdown: "# Net sales\n\n**EMEA** leads.",

  // The one entry that MUST produce no tokens: it is plain text by definition,
  // and a grammar that found something to highlight in it would be wrong.
  plaintext: "just words",
};

const UNHIGHLIGHTED = new Set(["plaintext"]);

describe("every language in the registry", () => {
  it("has a sample proving it renders", () => {
    // Adding a language without proving it highlights fails HERE rather than
    // shipping a tag that renders grey.
    const entries = Object.keys(LANGUAGE_TABLE).sort();
    expect(Object.keys(SAMPLES).sort()).toEqual(entries);
  });

  it.each(Object.keys(LANGUAGE_TABLE))("highlights %s", (id) => {
    const tokens = tokenCount(id, SAMPLES[id]!);
    if (UNHIGHLIGHTED.has(id)) expect(tokens).toBe(0);
    else expect(tokens, `${id} produced no highlight tokens`).toBeGreaterThan(0);
  });

  it.each(Object.keys(LANGUAGE_TABLE))("names %s and knows it", (id) => {
    const resolved = resolveLanguage(id);
    expect(resolved.known).toBe(true);
    expect(resolved.name).toBe(LANGUAGE_TABLE[id]!.name);
    // Never the raw tag echoed back, except where the tag IS the name (SQL, R).
    expect(resolved.name.length).toBeGreaterThan(0);
  });

  // Every ALIAS too, not just the canonical ids: an alias that highlights but
  // resolves to no name, or resolves to a name but does not highlight, is the
  // exact failure this registry exists to make impossible.
  it.each(knownLanguageTags())("resolves the tag %s to both a grammar and a name", (tag) => {
    expect(highlighter().registered(tag), `${tag} is not registered`).toBe(true);
    expect(resolveLanguage(tag).known, `${tag} has no name`).toBe(true);
  });
});

describe("a tag the registry does not know", () => {
  it("is named verbatim rather than replaced with a guess", () => {
    // The model wrote `zzql` for a reason the reader may recognize even when we
    // do not. Calling it "Code" would state something untrue about their code.
    expect(resolveLanguage("zzql")).toEqual({ name: "zzql", known: false });
    expect(resolveLanguage("  MalloyQL  ")).toEqual({ name: "MalloyQL", known: false });
  });

  it("gives an untagged fence a generic name and no grammar", () => {
    expect(resolveLanguage("")).toEqual({ name: "Code", known: false });
    expect(resolveLanguage("   ")).toEqual({ name: "Code", known: false });
  });

  it("folds case and surrounding space on a tag it does know", () => {
    expect(resolveLanguage(" SparkSQL ").name).toBe("Spark SQL");
  });
});

describe("the new grammars against hostile input", () => {
  // A grammar is fed untrusted model output on every token of a stream. These
  // are data-only definitions, so markup in a fence body is CLASSIFIED, never
  // interpreted: every element in the resulting tree is a highlighter span, and
  // the markup itself survives as text.
  const HOSTILE = [
    "<script>alert(1)</script>",
    '<img src=x onerror="alert(1)">',
    "javascript:alert(1)",
    "<!-- ]]> --><svg/onload=alert(1)>",
  ].join("\n");

  /** Every element the highlighter produced, and its property names. */
  function elements(node: unknown, found: Array<{ tag: string; props: string[] }> = []) {
    if (typeof node !== "object" || node === null) return found;
    const record = node as { type?: string; tagName?: string; properties?: object; children?: unknown[] };
    if (record.type === "element") {
      found.push({ tag: record.tagName ?? "", props: Object.keys(record.properties ?? {}) });
    }
    for (const child of record.children ?? []) elements(child, found);
    return found;
  }

  /** Concatenated text nodes — what the reader actually sees, and what Copy
   * yields. */
  function text(node: unknown): string {
    if (typeof node !== "object" || node === null) return "";
    const record = node as { type?: string; value?: string; children?: unknown[] };
    if (record.type === "text") return record.value ?? "";
    return (record.children ?? []).map(text).join("");
  }

  it.each(["dax", "kql", "m"])("emits only highlighter spans for a hostile %s fence", (tag) => {
    const produced = elements(highlighter().highlight(tag, HOSTILE));
    expect(produced.length).toBeGreaterThan(0);
    for (const element of produced) {
      // A grammar can only ever ask for a class name. No `script`, no `img`, no
      // event-handler property can come out of one.
      expect(element.tag).toBe("span");
      expect(element.props).toEqual(["className"]);
    }
  });

  it.each(["dax", "kql", "m"])("keeps a hostile %s fence's bytes intact as text", (tag) => {
    // The markup is not stripped either. Copy has to yield the fence body
    // byte-for-byte, so the dangerous-looking string must still be THERE —
    // as a text node, which is the whole distinction.
    expect(text(highlighter().highlight(tag, HOSTILE))).toBe(HOSTILE);
  });

  it("terminates on an unclosed string rather than consuming the rest", () => {
    // A streaming fence is routinely half-arrived. The grammar must not hang or
    // throw on it.
    for (const tag of ["dax", "kql", "m"]) {
      expect(() => highlighter().highlight(tag, 'x = "unterminated')).not.toThrow();
    }
  });
});
