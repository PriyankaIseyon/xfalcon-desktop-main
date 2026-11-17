import { describe, expect, it } from "vitest";
import { projectFileRendering } from "./projectFileRendering";

describe("projectFileRendering", () => {
  it.each([
    ["reports/Q3.html", { kind: "html" }],
    ["site/index.HTM", { kind: "html" }],
    ["page.xhtml", { kind: "html" }],
    ["charts/funnel.svg", { kind: "svg" }],
    ["README.md", { kind: "markdown" }],
    ["docs/guide.markdown", { kind: "markdown" }],
    ["data/regions.csv", { kind: "delimited", delimiter: "," }],
    ["data/regions.tsv", { kind: "delimited", delimiter: "\t" }],
    ["data/regions.tab", { kind: "delimited", delimiter: "\t" }],
    ["data/rows.json", { kind: "json" }],
    ["data/events.jsonl", { kind: "jsonl" }],
    ["data/events.ndjson", { kind: "jsonl" }],
  ])("renders %s as its chat type", (path, expected) => {
    expect(projectFileRendering(path)).toEqual(expected);
  });

  it.each([
    ["queries/pipeline.snowflake.sql", "sql"],
    ["model/measures.dax", "dax"],
    ["logs/query.kql", "kql"],
    ["etl/load.pq", "pq"],
    ["analysis/fit.py", "py"],
    ["analysis/fit.R", "r"],
    ["analysis/fit.jl", "julia"],
    ["dbt/models/schema.yml", "yml"],
    ["pyproject.toml", "toml"],
    ["setup.cfg", "cfg"],
    ["src/app.tsx", "tsx"],
    ["src/main.rs", "rs"],
    ["build.gradle.kts", "kotlin"],
    ["scripts/deploy.ps1", "ps1"],
    ["scripts/run.sh", "sh"],
    ["fix.patch", "patch"],
    ["notebooks/explore.ipynb", "json"],
    ["styles/site.scss", "scss"],
  ])("highlights %s with the registry tag %s", (path, tag) => {
    expect(projectFileRendering(path)).toEqual({ kind: "code", tag });
  });

  it.each([
    ["Dockerfile", "dockerfile"],
    ["deploy/Containerfile", "dockerfile"],
    ["Makefile", "makefile"],
    ["GNUmakefile", "makefile"],
  ])("recognizes the well-known name %s", (path, tag) => {
    expect(projectFileRendering(path)).toEqual({ kind: "code", tag });
  });

  it.each(["notes.txt", "server.log", "LICENSE", ".gitignore", "archive.tar.zzz", "trailing.", "solver.m"])(
    "leaves %s as plain text",
    (path) => {
      expect(projectFileRendering(path)).toEqual({ kind: "plain" });
    },
  );
});
