import assert from "node:assert/strict";
import { createHash, generateKeyPairSync, sign } from "node:crypto";
import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { createServer } from "node:http";
import { prepareDesktopRelease } from "./prepare-desktop-release.mjs";
import { compareVersions, publishStaging, verifyDelivery, validateManifest } from "./publish-desktop-staging.mjs";
const base = "https://updates.example.com";
const pointer = "desktop/channels/staging/latest.json";
async function fixture(t) {
  const input = await mkdtemp(path.join(os.tmpdir(), "staging-test-"));
  t.after(() => rm(input, { recursive: true, force: true }));
  for (const [name, body] of Object.entries({
    "app.exe": "windows",
    "app.exe.sig": "win-sig",
    "app.msi": "msi",
    "app.msi.sig": "msi-sig",
    "app.app.tar.gz": "mac",
    "app.app.tar.gz.sig": "mac-sig",
  }))
    await writeFile(path.join(input, name), body);
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const keyId = Buffer.from("12345678");
  const publicKeyText = Buffer.from(
    "untrusted comment: test\n" +
      Buffer.concat([
        Buffer.from("Ed"),
        keyId,
        publicKey.export({ type: "spki", format: "der" }).subarray(-32),
      ]).toString("base64") +
      "\n",
  ).toString("base64");
  for (const [name, body] of [
    ["app.exe", "windows"],
    ["app.msi", "msi"],
    ["app.app.tar.gz", "mac"],
  ]) {
    const signature = sign(null, createHash("blake2b512").update(body).digest(), privateKey);
    const trusted = "timestamp:1";
    const envelope = [
      "untrusted comment: test",
      Buffer.concat([Buffer.from("ED"), keyId, signature]).toString("base64"),
      "trusted comment: " + trusted,
      sign(null, Buffer.concat([signature, Buffer.from(trusted)]), privateKey).toString("base64"),
    ].join("\n");
    await writeFile(path.join(input, name + ".sig"), Buffer.from(envelope).toString("base64"));
  }
  const output = path.join(input, "output");
  await prepareDesktopRelease({
    input,
    output,
    repository: "owner/repo",
    version: "1.2.3",
    tag: "v1.2.3",
    deliveryBase: base,
  });
  const objects = new Map(),
    writes = [];
  const store = {
    identity: async () => "572186105468",
    get: async (key) => objects.get(key) ?? null,
    put: async (key, bytes, condition) => {
      const old = objects.get(key);
      if (condition.etag ? old?.etag !== condition.etag : old) throw new Error("PreconditionFailed");
      objects.set(key, { bytes, etag: createHash("sha256").update(bytes).digest("hex") });
      writes.push(key);
    },
  };
  const verified = [];
  const options = {
    publicKey: publicKeyText,
    version: "1.2.3",
    input: output,
    base,
    store,
    verifyDelivery: async (url, expected) => {
      const key = decodeURIComponent(new URL(url).pathname.slice(1));
      assert.equal(createHash("sha256").update(objects.get(key).bytes).digest("base64"), expected);
      verified.push(key);
    },
  };
  return { options, objects, writes, verified };
}
test("staging does not advertise; promotion verifies delivery and writes channel last", async (t) => {
  const f = await fixture(t);
  await publishStaging({ ...f.options, mode: "stage" });
  assert.equal(f.objects.has(pointer), false);
  await publishStaging({ ...f.options, mode: "stage" }); // exact-byte retry is safe
  await publishStaging({ ...f.options, mode: "promote" });
  assert.equal(f.writes.at(-1), pointer);
  assert.equal(f.verified.length, 7);
  assert.equal(JSON.parse(f.objects.get(pointer).bytes).version, "1.2.3");
  const writesBeforeRetry = f.writes.length;
  await publishStaging({ ...f.options, mode: "promote" });
  assert.equal(f.writes.length, writesBeforeRetry);
  assert.equal(f.verified.length, 14);
});
test("corrupted candidate and failed delivery cannot advertise an update", async (t) => {
  const f = await fixture(t);
  await publishStaging({ ...f.options, mode: "stage" });
  const key = "desktop/candidates/1.2.3/app.exe";
  const original = f.objects.get(key);
  f.objects.set(key, { bytes: Buffer.from("bad") });
  await assert.rejects(publishStaging({ ...f.options, mode: "promote" }), /checksum/);
  assert.equal(f.objects.has(pointer), false);
  f.objects.set(key, original);
  await assert.rejects(
    publishStaging({
      ...f.options,
      mode: "promote",
      verifyDelivery: async () => {
        throw new Error("offline");
      },
    }),
    /offline/,
  );
  assert.equal(f.objects.has(pointer), false);
});
test("wrong account and immutable changes fail closed", async (t) => {
  const f = await fixture(t);
  await assert.rejects(
    publishStaging({
      ...f.options,
      mode: "stage",
      store: { ...f.options.store, identity: async () => "812546850013" },
    }),
    /AWS account/,
  );
  assert.equal(f.writes.length, 0);
  await publishStaging({ ...f.options, mode: "stage" });
  await writeFile(path.join(f.options.input, "app.exe"), "different");
  await assert.rejects(publishStaging({ ...f.options, mode: "stage" }), /Invalid updater signature/);
});
test("concurrent promotion cannot overwrite an advanced channel", async (t) => {
  const f = await fixture(t);
  await publishStaging({ ...f.options, mode: "stage" });
  await assert.rejects(
    publishStaging({
      ...f.options,
      mode: "promote",
      verifyDelivery: async () => {
        f.objects.set(pointer, { bytes: Buffer.from('{"version":"1.2.4"}'), etag: "newer" });
      },
    }),
    /PreconditionFailed/,
  );
  assert.equal(JSON.parse(f.objects.get(pointer).bytes).version, "1.2.4");
});
test("numeric ordering prevents lexical downgrade and rejects traversal", () => {
  assert.equal(compareVersions("1.10.0", "1.9.0"), 1);
  assert.equal(compareVersions("1.2.3", "2.0.0"), -1);
  assert.throws(() => compareVersions("../1", "1.2.3"), /Invalid/);
});

test("forged candidate hashes cannot bypass pinned signature verification", async (t) => {
  const f = await fixture(t);
  await publishStaging({ ...f.options, mode: "stage" });
  const bytes = Buffer.from("forged installer");
  f.objects.set("desktop/candidates/1.2.3/app.exe", { bytes });
  const indexKey = "desktop/candidates/1.2.3/index.json";
  const index = JSON.parse(f.objects.get(indexKey).bytes);
  index.files.find((entry) => entry.name === "app.exe").sha256 = createHash("sha256").update(bytes).digest("base64");
  f.objects.set(indexKey, { bytes: Buffer.from(JSON.stringify(index)) });
  await assert.rejects(publishStaging({ ...f.options, mode: "promote" }), /Invalid updater signature/);
  assert.equal(f.objects.has(pointer), false);
  assert.equal(
    f.writes.some((key) => key.startsWith("desktop/releases/")),
    false,
  );
});

test("same-version different manifest and older versions cannot replace the channel", async (t) => {
  const f = await fixture(t);
  await publishStaging({ ...f.options, mode: "stage" });
  await publishStaging({ ...f.options, mode: "promote" });
  const original = f.objects.get(pointer);
  const conflict = JSON.parse(original.bytes);
  conflict.notes = "different release";
  f.objects.set(pointer, { bytes: Buffer.from(JSON.stringify(conflict)), etag: "conflict" });
  await assert.rejects(publishStaging({ ...f.options, mode: "promote" }), /Same-version/);
  assert.equal(JSON.parse(f.objects.get(pointer).bytes).notes, "different release");
  f.objects.set(pointer, original);
  await assert.rejects(publishStaging({ ...f.options, mode: "promote", version: "1.2.2" }), /downgrade/);
});

test("same-version retry still validates delivery before declaring success", async (t) => {
  const f = await fixture(t);
  await publishStaging({ ...f.options, mode: "stage" });
  await publishStaging({ ...f.options, mode: "promote" });
  await assert.rejects(
    publishStaging({
      ...f.options,
      mode: "promote",
      verifyDelivery: async () => {
        throw new Error("delivery unavailable");
      },
    }),
    /delivery unavailable/,
  );
});

async function deliveryServer(t, handler) {
  const server = createServer(handler);
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(
    () =>
      new Promise((resolve) => {
        server.closeAllConnections();
        server.close(resolve);
      }),
  );
  return `http://127.0.0.1:${server.address().port}/artifact?token=do-not-log`;
}

test("delivery retries transient HTTP failures then verifies streamed bytes", async (t) => {
  let attempts = 0;
  const url = await deliveryServer(t, (_request, response) => {
    attempts++;
    if (attempts < 3) {
      response.writeHead(attempts === 1 ? 503 : 429);
      response.end("retry");
    } else response.end("signed artifact");
  });
  await verifyDelivery(url, createHash("sha256").update("signed artifact").digest("base64"));
  assert.equal(attempts, 3);
});

test("delivery rejects checksum mismatches and redirects without retry or query disclosure", async (t) => {
  for (const status of [200, 302]) {
    let attempts = 0;
    const url = await deliveryServer(t, (_request, response) => {
      attempts++;
      response.writeHead(status, { Location: "/another" });
      response.end("wrong");
    });
    await assert.rejects(verifyDelivery(url, "expected"), (error) => {
      assert.match(error.message, status === 200 ? /checksum mismatch/ : /HTTP 302/);
      assert.equal(error.message.includes("do-not-log"), false);
      return true;
    });
    assert.equal(attempts, 1);
  }
});

test("delivery network retries are bounded and expose the connection error code", async (t) => {
  let attempts = 0;
  const url = await deliveryServer(t, (request) => {
    attempts++;
    request.socket.destroy();
  });
  await assert.rejects(verifyDelivery(url, "expected"), /after 3 attempt\(s\).*UND_ERR_SOCKET/);
  assert.equal(attempts, 3);
});

test("publisher refuses missing MSI mapping and cross-installer mappings", async (t) => {
  const f = await fixture(t);
  const manifest = JSON.parse(await readFile(path.join(f.options.input, "latest.json"), "utf8"));
  validateManifest(manifest, "1.2.3", base);
  const missing = structuredClone(manifest);
  delete missing.platforms["windows-x86_64-msi"];
  assert.throws(() => validateManifest(missing, "1.2.3", base), /Missing signed platform: windows-x86_64-msi/);
  for (const [target, source] of [
    ["windows-x86_64-msi", "windows-x86_64-nsis"],
    ["windows-x86_64-nsis", "windows-x86_64-msi"],
  ]) {
    const mixed = structuredClone(manifest);
    mixed.platforms[target] = mixed.platforms[source];
    assert.throws(() => validateManifest(mixed, "1.2.3", base), /Wrong installer format/);
  }
});
