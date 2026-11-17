import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

import { assertStagingDeliveryBase } from "./prepare-desktop-release.mjs";

import { verifyDesktopSignature } from "./verify-desktop-signature.mjs";

const BUCKET = "xfalcon-desktop-artifacts-staging";
const ACCOUNT = "572186105468";
const POINTER = "desktop/channels/staging/latest.json";
const digest = (bytes) => createHash("sha256").update(bytes).digest("base64");
const jsonBytes = (value) => Buffer.from(`${JSON.stringify(value, null, 2)}\n`);
export function compareVersions(left, right) {
  const parse = (v) => {
    if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(v)) throw new Error(`Invalid release version: ${v}`);
    return v.split(".").map(BigInt);
  };
  const a = parse(left),
    b = parse(right);
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] > b[i] ? 1 : -1;
  return 0;
}
export function validateManifest(manifest, version, base) {
  compareVersions(version, version);
  const origin = new URL(base);
  if (origin.protocol !== "https:" || origin.origin !== base || origin.username || origin.password)
    throw new Error("Delivery base must be an HTTPS origin without a trailing slash");
  if (manifest.version !== version) throw new Error("Manifest version mismatch");
  for (const platform of [
    "windows-x86_64",
    "windows-x86_64-nsis",
    "windows-x86_64-msi",
    "darwin-aarch64",
    "darwin-x86_64",
    "darwin-aarch64-app",
    "darwin-x86_64-app",
  ]) {
    const entry = manifest.platforms?.[platform];
    if (!entry || typeof entry.signature !== "string" || !entry.signature.trim())
      throw new Error(`Missing signed platform: ${platform}`);
    const prefix = `${base}/desktop/releases/${version}/`;
    if (!entry.url?.startsWith(prefix)) throw new Error("Unexpected artifact URL");
    const name = decodeURIComponent(entry.url.slice(prefix.length));
    if (!name || name !== path.basename(name) || /[\\/?#]/.test(name)) throw new Error("Invalid artifact name");
    const suffix = platform.endsWith("-msi")
      ? /\.msi(?:\.zip)?$/i
      : platform.startsWith("windows-")
        ? /(?:\.exe|\.nsis\.zip)$/i
        : /\.app\.(?:tar\.gz|zip)$/i;
    if (!suffix.test(name)) throw new Error(`Wrong installer format for ${platform}`);
  }
}

// AWS CLI supplies the checksum to S3, which verifies the uploaded bytes. The
// adapter keeps subprocess credentials outside repository configuration.
export function awsStore(directory) {
  const call = (args) =>
    JSON.parse(
      execFileSync("aws", [...args, "--region", "us-east-1", "--output", "json"], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      }) || "{}",
    );
  let sequence = 0;
  return {
    identity: async () => call(["sts", "get-caller-identity"]).Account,
    async get(key) {
      const file = path.join(directory, `get-${sequence++}`);
      try {
        const metadata = call([
          "s3api",
          "get-object",
          "--bucket",
          BUCKET,
          "--expected-bucket-owner",
          ACCOUNT,
          "--key",
          key,
          file,
        ]);
        return { bytes: await readFile(file), etag: metadata.ETag };
      } catch (error) {
        if (/\(NoSuchKey\)/.test(String(error.stderr))) return null;
        throw error;
      }
    },
    async put(key, bytes, condition, cacheControl) {
      const file = path.join(directory, `put-${sequence++}`);
      await writeFile(file, bytes);
      call([
        "s3api",
        "put-object",
        "--bucket",
        BUCKET,
        "--expected-bucket-owner",
        ACCOUNT,
        "--key",
        key,
        "--body",
        file,
        "--checksum-sha256",
        digest(bytes),
        "--cache-control",
        cacheControl,
        "--content-type",
        key.endsWith(".json") ? "application/json" : "application/octet-stream",
        ...(condition.etag ? ["--if-match", condition.etag] : ["--if-none-match", "*"]),
      ]);
    },
  };
}
async function immutable(store, key, bytes) {
  const existing = await store.get(key);
  if (existing) {
    if (digest(existing.bytes) !== digest(bytes)) throw new Error(`Immutable object differs: ${key}`);
    return;
  }
  await store.put(key, bytes, {}, "public,max-age=31536000,immutable");
}
export async function publishStaging({ mode, version, input, base, store, verifyDelivery, publicKey }) {
  compareVersions(version, version);
  if ((await store.identity()) !== ACCOUNT) throw new Error(`AWS account must be ${ACCOUNT}`);
  const prefix = `desktop/candidates/${version}/`;
  if (mode === "stage") {
    const manifest = JSON.parse(await readFile(path.join(input, "latest.json"), "utf8"));
    validateManifest(manifest, version, base);
    const files = (await readdir(input, { withFileTypes: true }))
      .filter((f) => f.isFile())
      .map((f) => f.name)
      .sort();
    if (files.includes("index.json")) throw new Error("index.json is reserved");
    const entries = [];
    for (const name of files) {
      const bytes = await readFile(path.join(input, name));
      entries.push({ name, sha256: digest(bytes) });
    }
    for (const entry of Object.values(manifest.platforms)) {
      const name = decodeURIComponent(new URL(entry.url).pathname.split("/").at(-1));
      if (!files.includes(name) || !files.includes(`${name}.sig`)) throw new Error(`Missing artifact pair: ${name}`);
      if ((await readFile(path.join(input, `${name}.sig`), "utf8")).trim() !== entry.signature)
        throw new Error("Signature does not match manifest");
      verifyDesktopSignature(await readFile(path.join(input, name)), entry.signature, publicKey);
    }
    for (const entry of entries)
      await immutable(store, prefix + entry.name, await readFile(path.join(input, entry.name)));
    await immutable(store, prefix + "index.json", jsonBytes({ version, files: entries }));
    return;
  }
  if (mode !== "promote") throw new Error("Mode must be stage or promote");
  const current = await store.get(POINTER);
  const ordering = current ? compareVersions(version, JSON.parse(current.bytes).version) : 1;
  if (ordering < 0) throw new Error("Promotion cannot downgrade the channel version");
  const indexObject = await store.get(prefix + "index.json");
  if (!indexObject) throw new Error("Candidate is incomplete or absent");
  const index = JSON.parse(indexObject.bytes);
  if (index.version !== version || !Array.isArray(index.files) || !index.files.length)
    throw new Error("Invalid candidate index");
  const files = new Map();
  for (const entry of index.files) {
    if (
      typeof entry.name !== "string" ||
      entry.name !== path.basename(entry.name) ||
      /[\\/]/.test(entry.name) ||
      files.has(entry.name)
    )
      throw new Error("Invalid candidate file name");
    const object = await store.get(prefix + entry.name);
    if (!object || digest(object.bytes) !== entry.sha256) throw new Error(`Candidate checksum mismatch: ${entry.name}`);
    files.set(entry.name, object.bytes);
  }
  const manifestBytes = files.get("latest.json");
  if (ordering === 0 && !current.bytes.equals(manifestBytes))
    throw new Error("Same-version channel manifest differs from candidate");
  const manifest = JSON.parse(manifestBytes);
  validateManifest(manifest, version, base);
  for (const entry of Object.values(manifest.platforms)) {
    const name = decodeURIComponent(new URL(entry.url).pathname.split("/").at(-1));
    if (!files.has(name) || files.get(`${name}.sig`)?.toString().trim() !== entry.signature)
      throw new Error("Candidate updater pair is incomplete");
    verifyDesktopSignature(files.get(name), entry.signature, publicKey);
  }
  for (const [name, bytes] of files) await immutable(store, `desktop/releases/${version}/${name}`, bytes);
  // Validate the actual client delivery route before advertising this version.
  for (const entry of index.files)
    await verifyDelivery(`${base}/desktop/releases/${version}/${encodeURIComponent(entry.name)}`, entry.sha256);
  if (ordering === 0) {
    const advertised = await store.get(POINTER);
    if (!advertised?.bytes.equals(manifestBytes)) throw new Error("Channel changed during retry verification");
    return;
  }
  await store.put(
    POINTER,
    files.get("latest.json"),
    current ? { etag: current.etag } : {},
    "no-cache,max-age=0,must-revalidate",
  );
}
export async function verifyDelivery(url, expected) {
  const parsed = new URL(url);
  const safeUrl = `${parsed.origin}${parsed.pathname}`;
  for (let attempt = 1; attempt <= 3; attempt++) {
    let failure;
    let retryable = false;
    try {
      const response = await fetch(url, { redirect: "manual", signal: AbortSignal.timeout(900_000) });
      if (!response.ok) {
        await response.body?.cancel();
        failure = `HTTP ${response.status}`;
        retryable = response.status === 408 || response.status === 429 || response.status >= 500;
      } else {
        const hash = createHash("sha256");
        for await (const chunk of response.body) hash.update(chunk);
        if (hash.digest("base64") !== expected) {
          failure = "checksum mismatch";
        } else return;
      }
    } catch (error) {
      const causes = [error, error.cause, ...(error.cause?.errors ?? [])];
      const codes = [
        ...new Set(
          causes
            .map((cause) => cause?.code ?? cause?.name)
            .filter((code) => typeof code === "string" && /^[A-Za-z0-9_]{1,80}$/.test(code)),
        ),
      ];
      failure = codes.join(", ") || "network error";
      retryable = codes.some((code) =>
        /^(ECONNRESET|ECONNREFUSED|EAI_AGAIN|ENOTFOUND|ENETUNREACH|EHOSTUNREACH|ETIMEDOUT|EPIPE|UND_ERR_CONNECT_TIMEOUT|UND_ERR_HEADERS_TIMEOUT|UND_ERR_BODY_TIMEOUT|UND_ERR_SOCKET|TimeoutError)$/.test(
          code,
        ),
      );
    }
    if (!retryable || attempt === 3)
      throw new Error(`Delivery failed after ${attempt} attempt(s): ${safeUrl}: ${failure}`);
    await delay(attempt * 1000);
  }
}
async function main() {
  const [mode, version, input] = process.argv.slice(2);
  await assertStagingDeliveryBase(process.env.DESKTOP_UPDATE_BASE_URL);
  const directory = await mkdtemp(path.join(os.tmpdir(), "xfalcon-publish-"));
  try {
    await publishStaging({
      mode,
      version,
      input,
      base: process.env.DESKTOP_UPDATE_BASE_URL,
      store: awsStore(directory),
      publicKey: JSON.parse(await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8")).plugins
        .updater.pubkey,
      verifyDelivery,
    });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href)
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
