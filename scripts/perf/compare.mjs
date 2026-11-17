// Before/after table for two labels written by run.mjs. Delta is (after - before) / before.
// usage: node scripts/perf/compare.mjs <labelA> <labelB>
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { LABEL_PATTERN } from "./args.mjs";

const [a, b] = process.argv.slice(2);
if (!a || !b || !LABEL_PATTERN.test(a) || !LABEL_PATTERN.test(b)) {
  console.error(
    "usage: node scripts/perf/compare.mjs <labelA> <labelB>\nlabels take letters, digits, '.', '_' and '-' (the names run.mjs wrote)",
  );
  process.exit(1);
}
const RESULTS = fileURLToPath(new URL("results/", import.meta.url));

function load(label, kind) {
  const file = join(RESULTS, `${label}.${kind}.json`);
  try {
    return JSON.parse(readFileSync(file, "utf8"));
  } catch (e) {
    console.error(`cannot read ${file}: ${e.message}. Run "npm run perf -- --dist <dir> --label ${label}" first.`);
    process.exit(1);
  }
}

const delta = (x, y) => {
  if (typeof x !== "number" || typeof y !== "number") return "";
  if (x === 0) return y === 0 ? "0%" : "n/a";
  const pct = Math.round((100 * (y - x)) / x);
  return `${pct > 0 ? "+" : ""}${pct}%`;
};
const cell = (v) => (v === undefined ? "-" : String(v));

// One table: rows are metric names; columns are the two labels and the change.
function table(title, rowsByName) {
  console.log(`== ${title}`);
  const width = Math.max(12, ...Object.keys(rowsByName).map((k) => k.length));
  for (const [name, [x, y]] of Object.entries(rowsByName)) {
    console.log(`  ${name.padEnd(width)} ${cell(x).padStart(10)} ${cell(y).padStart(10)} ${delta(x, y).padStart(7)}`);
  }
  console.log("");
}

console.log(`${a} (before) -> ${b} (after)\n`);

const startupA = load(a, "startup").medians;
const startupB = load(b, "startup").medians;
const startupKeys = new Set([...Object.keys(startupA), ...Object.keys(startupB)]);
table("startup (medians)", Object.fromEntries([...startupKeys].map((k) => [k, [startupA[k], startupB[k]]])));

for (const kind of ["screens", "screens-lat"]) {
  const sa = load(a, kind).screens;
  const sb = load(b, kind).screens;
  const rows = {};
  for (const screen of new Set([...Object.keys(sa), ...Object.keys(sb)])) {
    for (const m of ["paintMs", "contentMs", "settledMs", "busyMs", "nodes"]) {
      rows[`${screen} ${m}`] = [sa[screen]?.[m], sb[screen]?.[m]];
    }
  }
  table(
    kind === "screens" ? "screen opens (medians)" : "screen opens with simulated IPC latency 120+-80 ms (medians)",
    rows,
  );
}

const scA = load(a, "scenarios").results;
const scB = load(b, "scenarios").results;
const scRows = {};
for (const name of new Set([...Object.keys(scA), ...Object.keys(scB)])) {
  const ra = scA[name] ?? {};
  const rb = scB[name] ?? {};
  for (const m of new Set([...Object.keys(ra), ...Object.keys(rb)])) {
    if (m === "sample") continue;
    scRows[`${name} ${m}`] = [ra[m], rb[m]];
  }
}
table("scenarios (medians)", scRows);
