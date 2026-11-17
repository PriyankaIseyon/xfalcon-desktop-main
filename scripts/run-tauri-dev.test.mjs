import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { chmod, copyFile, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

for (const branch of ["staging", "main", "feature/example"]) {
  test(`local launcher selects the staging updater only on staging (${branch})`, async (t) => {
    const root = await mkdtemp(path.join(os.tmpdir(), "xfalcon-launch-"));
    t.after(() => rm(root, { recursive: true, force: true }));
    await mkdir(path.join(root, "scripts"));
    await mkdir(path.join(root, "bin"));
    await copyFile(new URL("./run-tauri-dev.mjs", import.meta.url), path.join(root, "scripts/run-tauri-dev.mjs"));
    execFileSync("git", ["init", `--initial-branch=${branch}`, root], { stdio: "pipe" });
    // Exercise the actual launcher; replace only the external build command.
    await writeFile(path.join(root, "capture.cjs"), "console.log(JSON.stringify(process.argv.slice(2)));\n");
    const windows = process.platform === "win32";
    const executable = path.join(root, "bin", windows ? "npx.cmd" : "npx");
    await writeFile(
      executable,
      windows
        ? `@echo off\r\n"${process.execPath}" "%~dp0..\\capture.cjs" %*\r\n`
        : `#!/bin/sh\nexec '${process.execPath.replaceAll("'", "'\\''")}' "$(dirname "$0")/../capture.cjs" "$@"\n`,
    );
    if (!windows) await chmod(executable, 0o755);
    const env = { ...process.env };
    const pathKey = Object.keys(env).find((key) => key.toLowerCase() === "path") ?? "PATH";
    env[pathKey] = `${path.join(root, "bin")}${path.delimiter}${env[pathKey] ?? ""}`;
    const result = spawnSync(process.execPath, [path.join(root, "scripts/run-tauri-dev.mjs")], {
      cwd: root,
      env,
      encoding: "utf8",
      timeout: 10_000,
    });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(
      JSON.parse(result.stdout.trim()),
      branch === "staging" ? ["tauri", "dev", "--config", "src-tauri/tauri.staging.conf.json"] : ["tauri", "dev"],
    );
  });
}
