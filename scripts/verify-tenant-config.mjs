import { readFileSync } from "node:fs";

// Verifies that a built bundle carries the tenant coordinates the dispatch
// asked for. Shared by the macOS and Windows checks in
// `.github/workflows/desktop-build.yml` so the two platforms cannot drift:
// macOS had a check and Windows had none, which is exactly how a Windows-only
// packaging regression ships unnoticed.
//
// Reads a path from argv rather than `require()`-ing it: `require()` on a bare
// relative path resolves as a module name, not a file, which once reported an
// empty organization for a bundle that was in fact correct.
//
// Never prints a secret. `--expect-secret` asserts only that a value is
// present, because the value itself would end up in a build log.
//
// Both platforms are checked identically. There was a `--forbid-mcp-client`
// mode that asserted macOS carried NO packaged client; macOS now packages one
// like Windows, so that mode had no caller and is gone.

const COORDINATE_FLAGS = {
  da_base_url: "--da-base-url",
  casdoor_issuer: "--casdoor-issuer",
  casdoor_client_id: "--casdoor-client-id",
  casdoor_organization: "--casdoor-organization",
};

/** @returns {{ path: string, coordinates: Record<string, string | null>, clientId: string, expectSecret: boolean }} */
function parseArgs(argv) {
  const [path] = argv;
  const flag = (name) => {
    const index = argv.indexOf(name);
    return index === -1 ? null : (argv[index + 1] ?? null);
  };
  return {
    path,
    coordinates: Object.fromEntries(Object.entries(COORDINATE_FLAGS).map(([key, name]) => [key, flag(name)])),
    clientId: (flag("--mcp-client-id") ?? "").trim(),
    expectSecret: flag("--expect-secret") === "1",
  };
}

function fail(message) {
  console.error(`::error::${message}`);
  process.exit(1);
}

function isHttpsOrigin(value) {
  let url;
  try {
    url = new URL(value.trim());
  } catch {
    return false;
  }
  return (
    url.protocol === "https:" &&
    Boolean(url.hostname) &&
    !url.username &&
    !url.password &&
    url.pathname === "/" &&
    !url.search &&
    !url.hash
  );
}

const { path, coordinates, clientId, expectSecret } = parseArgs(process.argv.slice(2));
if (!path) fail("verify-tenant-config: no bundle path was given");

let config;
try {
  config = JSON.parse(readFileSync(path, "utf8"));
} catch (cause) {
  // A missing or malformed resource is the failure this script exists to catch,
  // so it is reported as one rather than as an unhandled stack trace.
  fail(
    `${path} could not be read as tenant config (${cause.message}) -- installs of this build cannot authenticate against the tenant and will show an empty (DA-backed) MCP server list until this is fixed`,
  );
}

if (config === null || typeof config !== "object" || Array.isArray(config)) {
  fail(`${path} must contain a tenant config object`);
}

const found = (key) => String(config[key] ?? "").trim();

for (const [key, flag] of Object.entries(COORDINATE_FLAGS)) {
  const actual = config[key];
  if (typeof actual !== "string" || !actual.trim()) {
    fail(`${path} has no non-empty string ${key} -- installs of this build cannot sign in`);
  }

  const expected = coordinates[key];
  if (typeof expected !== "string" || !expected.trim()) {
    fail(`verify-tenant-config: ${flag} must carry the non-empty workflow dispatch value`);
  }

  if (actual !== expected) {
    fail(`${path} ${key} does not exactly match the workflow dispatch value`);
  }
}

for (const key of ["da_base_url", "casdoor_issuer"]) {
  if (!isHttpsOrigin(config[key])) {
    fail(`${path} ${key} must be an HTTPS origin with no credentials, path, query, or fragment`);
  }
}

// A dispatch that supplied a client and a bundle that lost it look identical
// until a user tries to sign in, on either platform.
if (clientId && found("mcp_client_id") !== clientId) {
  fail(`${path} lost its mcp_client_id -- every user of this build will be asked to paste one`);
}

// Half a confidential client is worse than none: the id is accepted and the
// exchange then fails on the missing secret.
if (found("mcp_client_id") && expectSecret && !found("mcp_client_secret")) {
  fail(`${path} carries mcp_client_id but not the secret configured for it`);
}

if (!found("mcp_client_id") && found("mcp_client_secret")) {
  fail(`${path} carries an mcp_client_secret with no mcp_client_id -- it belongs to no client and must not ship`);
}

const client = found("mcp_client_id")
  ? found("mcp_client_secret")
    ? "confidential MCP fallback client"
    : "public MCP fallback client"
  : "no MCP fallback client";
console.log(`ok: ${path} carries a valid tenant-config.json for ${config.casdoor_organization.trim()} (${client})`);
console.log(`    keys present: ${Object.keys(config).join(", ")}`);
