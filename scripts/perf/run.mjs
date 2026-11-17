// Runs the whole suite against one built dist and writes results/<label>.*.json.
// usage: node scripts/perf/run.mjs [--dist dist] --label <name> [--runs 9]
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { arg, intArg, LABEL_PATTERN, usageError } from "./args.mjs";
import { resolveDist } from "./serve.mjs";

const label = arg("label");
if (!label || !LABEL_PATTERN.test(label))
  usageError(
    "usage: node scripts/perf/run.mjs [--dist dist] --label <name> [--runs 9]\n--label takes letters, digits, '.', '_' and '-'",
  );
const dist = resolveDist(arg("dist", "dist"));
const runs = String(intArg("runs", 9, 1));

const HERE = fileURLToPath(new URL(".", import.meta.url));
const RESULTS = fileURLToPath(new URL("results/", import.meta.url));
mkdirSync(RESULTS, { recursive: true });
const out = (kind) => join(RESULTS, `${label}.${kind}.json`);
const SCENARIOS =
  "open200,open10k,scroll10k,sidebar1500,filter20k,csvOpenFilter,palette2k,streamHtml,newChatSend,scrollHeavy";

const steps = [
  ["startup", "startup.mjs", ["--runs", runs, "--label", label, "--json", out("startup")]],
  ["screens", "screens.mjs", ["--runs", runs, "--label", label, "--json", out("screens")]],
  [
    "screens with IPC latency",
    "screens.mjs",
    ["--runs", runs, "--ipcDelay", "120", "--ipcJitter", "80", "--label", `${label}-lat`, "--json", out("screens-lat")],
  ],
  ["scenarios", "scenarios.mjs", ["--runs", runs, "--only", SCENARIOS, "--label", label, "--json", out("scenarios")]],
];

// A stop signal is forwarded to the running step, and run.mjs exits once that step has ended.
let child;
let stopping = null;
const relay = (signal) => {
  stopping = signal;
  child?.kill(signal);
};
process.on("SIGINT", relay);
process.on("SIGTERM", relay);

for (const [name, script, args] of steps) {
  if (stopping) break;
  console.log(`== ${name}`);
  child = spawn(process.execPath, [join(HERE, script), "--dist", dist, ...args], { stdio: "inherit" });
  const [code, signal] = await once(child, "exit");
  child = undefined;
  if (stopping) break;
  if (code !== 0) {
    console.error(`${name} failed (exit ${code ?? signal})`);
    process.exit(code ?? 1);
  }
}
if (stopping) {
  console.error(`stopped by ${stopping}`);
  process.exit(stopping === "SIGINT" ? 130 : 143);
}

// Unknown commands resolved to null by the mock, across all four result files. Reported, not fatal.
const unknown = new Set();
for (const kind of ["startup", "screens", "screens-lat", "scenarios"])
  for (const c of JSON.parse(readFileSync(out(kind), "utf8")).unknownCommands ?? []) unknown.add(c);
if (unknown.size)
  console.warn(
    `warning: the mock does not handle these native commands (they resolved to null): ${[...unknown].sort().join(", ")}`,
  );
console.log(`results written to ${RESULTS}${label}.*.json`);
