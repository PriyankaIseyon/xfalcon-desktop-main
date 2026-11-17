import { readdirSync } from "node:fs";
import { join } from "node:path";

// Owner decision 2026-08-06 (reverses the Task-5 "woff2 subsets only" rule):
// Google Sans ships as the ORIGINAL Google variable TTF pair, unmodified —
// a desktop installer has no page-weight budget, and the unmodified files
// remove subsetting as a fidelity variable. Geist Mono stays woff2.
const fontsDir = join(process.cwd(), "public", "fonts");
const entries = readdirSync(fontsDir);

const required = [
  "GoogleSans-Variable.ttf",
  "GoogleSans-Italic.ttf",
  "GeistMono.woff2",
  "GoogleSans-OFL.txt",
  "Geist-OFL.txt",
];

// Stray Google Sans woff2 subsets would silently shadow the originals in a
// future refactor — keep the directory unambiguous.
const staleSubsets = entries.filter((name) => /^GoogleSans.*\.woff2$/i.test(name));
if (staleSubsets.length > 0) {
  console.error("Font inventory check failed: stale Google Sans woff2 subsets:", staleSubsets.join(", "));
  process.exit(1);
}

const missing = required.filter((name) => !entries.includes(name));
if (missing.length > 0) {
  console.error("Font inventory check failed: missing required files:", missing.join(", "));
  process.exit(1);
}

console.log("Font inventory check passed.");
