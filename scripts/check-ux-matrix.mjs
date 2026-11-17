// The closure ledger has to be able to lose a row loudly.
//
// `ux-matrix.md` is a change's evidence base: its acceptance gate reads "every
// P01–P42 row is Verified", and the summary table states how many rows there
// are. Seventeen primitive rows once vanished in an unrelated edit to the same
// file. Nothing failed. The gate still said "every row", the table still
// parsed, and the seventeen primitives simply stopped being tracked -- the one
// failure a ledger must not have, because it reads as coverage.
//
// This checks the file against itself: the stated counts, the id sequence, and
// the status vocabulary the file declares.

import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join } from "node:path";

const changesDir = join(process.cwd(), "openspec", "changes");

/** Rows of one pipe table column, for ids shaped like `P07` or `S140`. */
function ids(source, prefix) {
  const pattern = new RegExp(`^\\| (${prefix}\\d+) `, "gm");
  return [...source.matchAll(pattern)].map((match) => match[1]);
}

/** The number a summary row states, e.g. `| Shared visible primitives | 42 |`. */
function statedCount(source, label) {
  const row = new RegExp(`^\\|\\s*${label}\\s*\\|\\s*(\\d+)\\s*\\|`, "m").exec(source);
  return row ? Number(row[1]) : null;
}

const failures = [];

function check(file, source) {
  // The file declares its own status vocabulary; nothing may invent a word.
  const ladder = /`((?:\w+ → )+\w+)`/.exec(source);
  const allowed = new Set((ladder ? ladder[1].split(" → ") : []).concat(["Blocked", "N/A", "Audited"]));

  for (const [prefix, label, contiguous] of [
    ["P", "Shared visible primitives", true],
    ["S", "Tracked user-visible surface rows", false],
  ]) {
    const found = ids(source, prefix);
    const stated = statedCount(source, label);
    if (stated === null) {
      failures.push(`${file}: no summary row states "${label}"`);
      continue;
    }
    if (found.length !== stated) {
      failures.push(
        `${file}: "${label}" states ${stated}, the table holds ${found.length}` +
          (found.length < stated ? " -- rows were dropped" : " -- the count is stale"),
      );
    }
    const duplicates = found.filter((id, index) => found.indexOf(id) !== index);
    if (duplicates.length > 0) {
      failures.push(`${file}: duplicate ${prefix} ids: ${[...new Set(duplicates)].join(", ")}`);
    }
    if (contiguous) {
      const width = found[0]?.length - 1;
      const gaps = found
        .map((id, index) => [id, `${prefix}${String(index + 1).padStart(width, "0")}`])
        .filter(([actual, expected]) => actual !== expected);
      if (gaps.length > 0) {
        failures.push(`${file}: ${prefix} ids are not contiguous: first break at ${gaps[0][0]}`);
      }
    }
  }

  // The status column is not the last column in every table, so its index is
  // read from each table's own header rather than assumed.
  if (allowed.size > 3) {
    let statusIndex = -1;
    for (const line of source.split("\n")) {
      const cells = line.startsWith("|")
        ? line
            .split("|")
            .slice(1, -1)
            .map((cell) => cell.trim())
        : null;
      if (cells === null) continue;
      if (cells[0] === "ID") {
        statusIndex = cells.indexOf("Status");
        continue;
      }
      if (statusIndex < 0 || !/^[PS]\d+$/.test(cells[0] ?? "")) continue;
      const status = (cells[statusIndex] ?? "").split(";")[0].trim();
      if (!allowed.has(status)) {
        failures.push(`${file}: ${cells[0]} has status "${status}", which the ladder does not declare`);
      }
    }
  }
}

if (existsSync(changesDir)) {
  for (const change of readdirSync(changesDir)) {
    const file = join(changesDir, change, "ux-matrix.md");
    if (!existsSync(file)) continue;
    check(join("openspec", "changes", change, "ux-matrix.md"), readFileSync(file, "utf8"));
  }
}

if (failures.length > 0) {
  console.error("UX matrix check failed:");
  for (const failure of failures) console.error(`  ${failure}`);
  process.exit(1);
}

console.log("UX matrix check passed.");
