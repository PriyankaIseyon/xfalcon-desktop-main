import { execFileSync, spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const branch = execFileSync("git", ["branch", "--show-current"], { cwd: root, encoding: "utf8" }).trim();
const args = ["tauri", "dev"];
if (branch === "staging") args.push("--config", "src-tauri/tauri.staging.conf.json");

const env = { ...process.env };
delete env.CARGO_TARGET_DIR;
delete env.npm_config_devdir;
delete env.NPM_CONFIG_DEVDIR;

const child = spawn("npx", args, {
  cwd: root,
  env,
  stdio: "inherit",
  shell: true,
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});
