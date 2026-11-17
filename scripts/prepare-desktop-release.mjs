import { copyFile, mkdir, readdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const RELEASE_FILE = /(?:\.dmg|\.exe|\.msi|\.zip|\.tar\.gz|\.sig|\.cdx\.json)$/i;
// Must match desktop_update.rs: clients reject larger downloads before install.
const MAX_UPDATE_BYTES = 512 * 1024 * 1024;

async function filesBelow(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const resolved = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await filesBelow(resolved)));
    else if (entry.isFile()) files.push(resolved);
  }
  return files;
}

function updaterAsset(assets, suffixes) {
  for (const suffix of suffixes) {
    const signature = assets.find((file) => path.basename(file).toLowerCase().endsWith(suffix));
    if (!signature) continue;
    const archiveName = path.basename(signature).slice(0, -4);
    const archive = assets.find((file) => path.basename(file) === archiveName);
    if (archive) return { archive, signature };
  }
  throw new Error(`Missing updater pair for ${suffixes.join(", ")}`);
}

export async function assertStagingDeliveryBase(base) {
  const config = JSON.parse(await readFile(new URL("../src-tauri/tauri.staging.conf.json", import.meta.url), "utf8"));
  const endpoints = config.plugins?.updater?.endpoints;
  if (!Array.isArray(endpoints) || endpoints.length !== 1 || base !== new URL(endpoints[0]).origin) {
    throw new Error("DESKTOP_UPDATE_BASE_URL must match the compiled staging updater origin");
  }
}

function downloadUrl(repository, tag, file, deliveryBase, version) {
  if (deliveryBase)
    return `${deliveryBase}/desktop/releases/${encodeURIComponent(version)}/${encodeURIComponent(path.basename(file))}`;
  return `https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(path.basename(file))}`;
}

export async function prepareDesktopRelease({
  input,
  output,
  repository,
  tag,
  version,
  publishedAt,
  deliveryBase,
  notes = "",
}) {
  for (const [name, value] of Object.entries({ input, output, repository, tag, version })) {
    if (typeof value !== "string" || !value.trim()) throw new Error(`Missing required option: ${name}`);
  }
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) throw new Error("Invalid GitHub repository name");
  if (tag !== `v${version}`) throw new Error("Release tag must equal v plus the application version");

  if (deliveryBase) {
    const url = new URL(deliveryBase);
    if (
      url.protocol !== "https:" ||
      url.username ||
      url.password ||
      url.search ||
      url.hash ||
      url.pathname !== "/" ||
      url.port
    ) {
      throw new Error("Delivery base must be an HTTPS origin");
    }
    deliveryBase = url.origin;
  }
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version))
    throw new Error("Release version must be numeric major.minor.patch");

  const allFiles = (await filesBelow(input)).filter((file) => RELEASE_FILE.test(path.basename(file)));
  if (allFiles.some((file) => /\.zip(?:\.sig)?$/i.test(file))) {
    throw new Error("ZIP updater archives are unsupported; use Windows EXE/MSI and macOS app.tar.gz packages");
  }
  const byName = new Map();
  for (const file of allFiles) {
    const name = path.basename(file);
    if (byName.has(name)) throw new Error(`Duplicate release asset name: ${name}`);
    byName.set(name, file);
  }
  const assets = [...byName.values()];
  const windows = updaterAsset(assets, [".exe.sig"]);
  const msi = updaterAsset(assets, [".msi.sig"]);
  const macos = updaterAsset(assets, [".app.tar.gz.sig"]);
  for (const { archive } of [windows, msi, macos]) {
    const { size } = await stat(archive);
    if (size === 0 || size > MAX_UPDATE_BYTES) {
      throw new Error(
        `Update artifact must be nonempty and at most 512 MiB: ${path.basename(archive)} (${size} bytes)`,
      );
    }
  }
  const windowsEntry = {
    signature: (await readFile(windows.signature, "utf8")).trim(),
    url: downloadUrl(repository, tag, windows.archive, deliveryBase, version),
  };
  const msiEntry = {
    signature: (await readFile(msi.signature, "utf8")).trim(),
    url: downloadUrl(repository, tag, msi.archive, deliveryBase, version),
  };
  const macosEntry = {
    signature: (await readFile(macos.signature, "utf8")).trim(),
    url: downloadUrl(repository, tag, macos.archive, deliveryBase, version),
  };
  if (!windowsEntry.signature || !msiEntry.signature || !macosEntry.signature)
    throw new Error("Updater signature is empty");

  await mkdir(output, { recursive: true });
  for (const file of assets) await copyFile(file, path.join(output, path.basename(file)));
  const manifest = {
    version,
    notes,
    pub_date: publishedAt ?? new Date().toISOString(),
    platforms: {
      "darwin-aarch64": macosEntry,
      "darwin-x86_64": macosEntry,
      "darwin-aarch64-app": macosEntry,
      "darwin-x86_64-app": macosEntry,
      "windows-x86_64": windowsEntry,
      "windows-x86_64-nsis": windowsEntry,
      "windows-x86_64-msi": msiEntry,
    },
  };
  await writeFile(path.join(output, "latest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  return manifest;
}

async function main() {
  const options = Object.fromEntries(
    process.argv.slice(2).map((value) => {
      const separator = value.indexOf("=");
      if (separator < 3) throw new Error(`Invalid option: ${value}`);
      return [value.slice(0, separator), value.slice(separator + 1)];
    }),
  );
  await prepareDesktopRelease({
    input: options["--input"],
    output: options["--output"],
    repository: options["--repository"],
    tag: options["--tag"],
    version: options["--version"],
    deliveryBase: options["--delivery-base"],
    notes: options["--notes-file"] ? await readFile(options["--notes-file"], "utf8") : "",
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
