import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { access, mkdtemp, mkdir, readFile, rm, truncate, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { prepareDesktopRelease, assertStagingDeliveryBase } from "./prepare-desktop-release.mjs";

for (const archive of ["app.exe", "app.msi", "app.app.tar.gz"]) {
  for (const size of [0, 512 * 1024 * 1024 + 1]) {
    test(`rejects ${archive} with ${size} bytes before preparing a release`, async (t) => {
      const root = await mkdtemp(path.join(os.tmpdir(), "xfalcon-release-size-"));
      t.after(() => rm(root, { recursive: true, force: true }));
      const input = path.join(root, "input");
      const output = path.join(root, "output");
      await mkdir(input);
      for (const name of ["app.exe", "app.msi", "app.app.tar.gz"]) {
        await writeFile(path.join(input, name), "artifact");
        await writeFile(path.join(input, `${name}.sig`), "signature");
      }
      await truncate(path.join(input, archive), size);
      await assert.rejects(
        prepareDesktopRelease({ input, output, repository: "owner/repo", tag: "v1.2.3", version: "1.2.3" }),
        /Update artifact must be nonempty and at most 512 MiB/,
      );
      await assert.rejects(access(output), { code: "ENOENT" });
    });
  }
}

test("maps NSIS and MSI clients to their own signed installer", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "xfalcon-release-"));
  const input = path.join(root, "input");
  const output = path.join(root, "output");
  await mkdir(path.join(input, "windows"), { recursive: true });
  await mkdir(path.join(input, "macos"), { recursive: true });
  const files = {
    "windows/app.exe": "nsis",
    "windows/app.exe.sig": "nsis-signature",
    "windows/app.msi": "msi",
    "windows/app.msi.sig": "msi-signature",
    "macos/app.app.tar.gz": "mac",
    "macos/app.app.tar.gz.sig": "mac-signature",
    "windows/windows.cdx.json": "{}",
    "macos/macos.cdx.json": "{}",
  };
  for (const [relative, contents] of Object.entries(files)) {
    await writeFile(path.join(input, relative), contents);
  }

  const manifest = await prepareDesktopRelease({
    input,
    output,
    repository: "owner/repo",
    tag: "v1.2.3",
    version: "1.2.3",
    publishedAt: "2026-08-31T00:00:00.000Z",
  });

  assert.equal(manifest.platforms["windows-x86_64"].signature, "nsis-signature");
  assert.equal(
    manifest.platforms["windows-x86_64-nsis"].url,
    "https://github.com/owner/repo/releases/download/v1.2.3/app.exe",
  );
  assert.equal(manifest.platforms["windows-x86_64-msi"].signature, "msi-signature");
  assert.equal(
    manifest.platforms["windows-x86_64-msi"].url,
    "https://github.com/owner/repo/releases/download/v1.2.3/app.msi",
  );
  assert.equal(manifest.platforms["darwin-aarch64"].signature, "mac-signature");
  assert.equal(manifest.platforms["darwin-x86_64-app"].signature, "mac-signature");
  assert.deepEqual(JSON.parse(await readFile(path.join(output, "latest.json"), "utf8")), manifest);
  assert.equal(manifest.notes, "");

  const notes = "# Release notes\n\n- Smoother chat.\n- Optional suggestions.\n";
  const notesFile = path.join(root, "notes.md");
  await writeFile(notesFile, notes);
  execFileSync(process.execPath, [
    fileURLToPath(new URL("./prepare-desktop-release.mjs", import.meta.url)),
    `--input=${input}`,
    `--output=${output}`,
    "--repository=owner/repo",
    "--tag=v1.2.3",
    "--version=1.2.3",
    `--notes-file=${notesFile}`,
  ]);
  assert.equal(JSON.parse(await readFile(path.join(output, "latest.json"), "utf8")).notes, notes);
});

for (const archive of ["app.nsis.zip", "app.msi.zip", "app.app.zip"]) {
  test(`rejects unsupported ${archive} before preparing a release`, async (t) => {
    const root = await mkdtemp(path.join(os.tmpdir(), "xfalcon-release-zip-"));
    t.after(() => rm(root, { recursive: true, force: true }));
    const input = path.join(root, "input");
    const output = path.join(root, "output");
    await mkdir(input);
    for (const name of ["app.exe", "app.msi", "app.app.tar.gz", archive]) {
      await writeFile(path.join(input, name), "artifact");
      await writeFile(path.join(input, `${name}.sig`), "signature");
    }
    await assert.rejects(
      prepareDesktopRelease({ input, output, repository: "owner/repo", tag: "v1.2.3", version: "1.2.3" }),
      /ZIP updater archives are unsupported/,
    );
    await assert.rejects(access(output), { code: "ENOENT" });
  });
}

test("fails closed when one platform updater is absent", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "xfalcon-release-"));
  await writeFile(path.join(root, "app.exe"), "nsis");
  await writeFile(path.join(root, "app.exe.sig"), "signature");
  await assert.rejects(
    prepareDesktopRelease({
      input: root,
      output: path.join(root, "output"),
      repository: "owner/repo",
      tag: "v1.2.3",
      version: "1.2.3",
    }),
    /Missing updater pair/,
  );
});

test("fails closed on duplicate release asset names", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "xfalcon-release-"));
  await mkdir(path.join(root, "one"));
  await mkdir(path.join(root, "two"));
  await writeFile(path.join(root, "one", "app.exe"), "one");
  await writeFile(path.join(root, "two", "app.exe"), "two");
  await assert.rejects(
    prepareDesktopRelease({
      input: root,
      output: path.join(root, "output"),
      repository: "owner/repo",
      tag: "v1.2.3",
      version: "1.2.3",
    }),
    /Duplicate release asset name/,
  );
});

test("staging delivery uses application version and preserves existing release tags", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "xfalcon-staging-release-"));
  for (const [name, body] of Object.entries({
    "app.exe": "windows",
    "app.exe.sig": "signature",
    "app.msi": "msi",
    "app.msi.sig": "msi-signature",
    "app.app.tar.gz": "macos",
    "app.app.tar.gz.sig": "signature",
  }))
    await writeFile(path.join(root, name), body);
  const options = {
    input: root,
    output: path.join(root, "output"),
    repository: "owner/repo",
    tag: "v1.2.3",
    version: "1.2.3",
  };
  const github = await prepareDesktopRelease(options);
  assert.equal(github.version, "1.2.3");
  assert.equal(
    github.platforms["windows-x86_64"].url,
    "https://github.com/owner/repo/releases/download/v1.2.3/app.exe",
  );
  // Keep the second output outside the source tree to avoid copying prior artifacts.
  const deliveryInput = options.output;
  const delivery = await prepareDesktopRelease({
    ...options,
    input: deliveryInput,
    output: path.join(root, "delivery"),
    deliveryBase: "https://updates.example.com",
    notes: "- Smoother chat and optional suggestions.\n",
  });
  assert.equal(delivery.platforms["windows-x86_64"].url, "https://updates.example.com/desktop/releases/1.2.3/app.exe");
  assert.equal(
    delivery.platforms["windows-x86_64-msi"].url,
    "https://updates.example.com/desktop/releases/1.2.3/app.msi",
  );
  assert.equal(delivery.platforms["windows-x86_64-msi"].signature, "msi-signature");
  assert.equal(delivery.notes, "- Smoother chat and optional suggestions.\n");
});

test("staging origin must match the compiled endpoint", async () => {
  const config = JSON.parse(await readFile(new URL("../src-tauri/tauri.staging.conf.json", import.meta.url), "utf8"));
  const origin = new URL(config.plugins.updater.endpoints[0]).origin;
  await assertStagingDeliveryBase(origin);
  for (const value of [undefined, "https://replacement.cloudfront.net", origin + "/", origin + "/desktop"]) {
    await assert.rejects(assertStagingDeliveryBase(value), /must match the compiled/);
  }
});

test("an unsigned MSI cannot silently fall back to NSIS", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "xfalcon-release-"));
  for (const [name, value] of Object.entries({
    "app.exe": "nsis",
    "app.exe.sig": "nsis-signature",
    "app.msi": "msi",
    "app.app.tar.gz": "mac",
    "app.app.tar.gz.sig": "mac-signature",
  }))
    await writeFile(path.join(root, name), value);
  await assert.rejects(
    prepareDesktopRelease({
      input: root,
      output: path.join(root, "output"),
      repository: "owner/repo",
      tag: "v1.2.3",
      version: "1.2.3",
    }),
    /Missing updater pair.*msi/,
  );
});
