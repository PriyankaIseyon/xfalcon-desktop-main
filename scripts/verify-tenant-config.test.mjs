import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import assert from "node:assert/strict";

const script = fileURLToPath(new URL("./verify-tenant-config.mjs", import.meta.url));

const BASE = {
  da_base_url: "https://da.example.com",
  casdoor_issuer: "https://auth.example.com",
  casdoor_client_id: "client-123",
  casdoor_organization: "acme",
};

function run(config, args) {
  const dir = mkdtempSync(join(tmpdir(), "tenant-config-"));
  const path = join(dir, "tenant-config.json");
  if (config !== null) writeFileSync(path, JSON.stringify(config));
  const result = spawnSync(process.execPath, [script, path, ...args], { encoding: "utf8" });
  return { code: result.status, out: result.stdout, err: result.stderr };
}

function coordinateArgs(overrides = {}) {
  const expected = { ...BASE, ...overrides };
  return [
    "--da-base-url",
    expected.da_base_url,
    "--casdoor-issuer",
    expected.casdoor_issuer,
    "--casdoor-client-id",
    expected.casdoor_client_id,
    "--casdoor-organization",
    expected.casdoor_organization,
  ];
}

const coordinates = coordinateArgs();

test("a bundle carrying the dispatched coordinates passes", () => {
  const { code, out } = run({ ...BASE, mcp_client_id: "mcp-1", mcp_client_secret: "s3cret" }, [
    ...coordinates,
    "--mcp-client-id",
    "mcp-1",
    "--expect-secret",
    "1",
  ]);
  assert.equal(code, 0);
  assert.match(out, /confidential MCP fallback client/);
});

// The secret must never reach a build log; only whether one is present.
test("the secret value is never printed", () => {
  const { out, err } = run({ ...BASE, mcp_client_id: "mcp-1", mcp_client_secret: "s3cret" }, [
    ...coordinates,
    "--mcp-client-id",
    "mcp-1",
    "--expect-secret",
    "1",
  ]);
  assert.ok(!out.includes("s3cret"), out);
  assert.ok(!err.includes("s3cret"), err);
});

test("a lost client id fails the build rather than shipping a paste prompt", () => {
  const { code, err } = run(BASE, [...coordinates, "--mcp-client-id", "mcp-1"]);
  assert.equal(code, 1);
  assert.match(err, /lost its mcp_client_id/);
});

test("a configured secret that did not reach the bundle fails the build", () => {
  const { code, err } = run({ ...BASE, mcp_client_id: "mcp-1" }, [
    ...coordinates,
    "--mcp-client-id",
    "mcp-1",
    "--expect-secret",
    "1",
  ]);
  assert.equal(code, 1);
  assert.match(err, /not the secret configured for it/);
});

test("a secret with no client id must not ship", () => {
  const { code, err } = run({ ...BASE, mcp_client_secret: "orphan" }, coordinates);
  assert.equal(code, 1);
  assert.match(err, /belongs to no client/);
});

test("a wrong organization fails the build", () => {
  const { code, err } = run({ ...BASE, casdoor_organization: "other" }, coordinates);
  assert.equal(code, 1);
  assert.match(err, /casdoor_organization does not exactly match the workflow dispatch value/);
});

for (const key of Object.keys(BASE)) {
  test(`a missing or non-string ${key} fails the build`, () => {
    const { code, err } = run({ ...BASE, [key]: key === "casdoor_client_id" ? 42 : "  " }, coordinates);
    assert.equal(code, 1);
    assert.match(err, new RegExp(`no non-empty string ${key}`));
  });

  test(`a changed ${key} fails the exact dispatch comparison`, () => {
    const { code, err } = run({ ...BASE, [key]: `${BASE[key]}-changed` }, coordinates);
    assert.equal(code, 1);
    assert.match(err, new RegExp(`${key} does not exactly match the workflow dispatch value`));
  });
}

for (const [name, value] of [
  ["HTTP", "http://da.example.com"],
  ["credentials", "https://user:password@da.example.com"],
  ["path", "https://da.example.com/api"],
  ["query", "https://da.example.com/?tenant=acme"],
  ["fragment", "https://da.example.com/#tenant"],
]) {
  test(`a DA URL with ${name} is not a valid HTTPS origin`, () => {
    const expected = coordinateArgs({ da_base_url: value });
    const { code, err } = run({ ...BASE, da_base_url: value }, expected);
    assert.equal(code, 1);
    assert.match(err, /da_base_url must be an HTTPS origin/);
  });
}

test("the Casdoor issuer must also be an HTTPS origin", () => {
  const value = "https://auth.example.com/application/acme";
  const { code, err } = run({ ...BASE, casdoor_issuer: value }, coordinateArgs({ casdoor_issuer: value }));
  assert.equal(code, 1);
  assert.match(err, /casdoor_issuer must be an HTTPS origin/);
});

test("every expected workflow dispatch coordinate is required", () => {
  const { code, err } = run(BASE, coordinates.slice(0, -2));
  assert.equal(code, 1);
  assert.match(err, /--casdoor-organization must carry the non-empty workflow dispatch value/);
});

// The regression this whole check exists for: the resource never reached the
// bundle at all. It must report that, not crash with a stack trace.
test("a missing resource fails with a diagnosis rather than a stack trace", () => {
  const { code, err } = run(null, coordinates);
  assert.equal(code, 1);
  assert.match(err, /could not be read as tenant config/);
  assert.ok(!err.includes("at Object.readFileSync"), err);
});

test("a JSON value that is not a config object fails cleanly", () => {
  const dir = mkdtempSync(join(tmpdir(), "tenant-config-value-"));
  const path = join(dir, "tenant-config.json");
  writeFileSync(path, "null");
  const result = spawnSync(process.execPath, [script, path, ...coordinates], { encoding: "utf8" });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /must contain a tenant config object/);
  assert.ok(!result.stderr.includes("TypeError"), result.stderr);
});

test("a build with no MCP client configured is valid", () => {
  const { code, out } = run(BASE, coordinates);
  assert.equal(code, 0);
  assert.match(out, /no MCP fallback client/);
});

test("a bundle that carries the dispatched confidential client passes on either platform", () => {
  const { code, out } = run({ ...BASE, mcp_client_id: "mcp-1", mcp_client_secret: "s3cret" }, [
    ...coordinates,
    "--mcp-client-id",
    "mcp-1",
    "--expect-secret",
    "1",
  ]);
  assert.equal(code, 0);
  assert.match(out, /confidential MCP fallback client/);
  // The value itself would end up in a build log anyone with Actions access
  // can read.
  assert.ok(!out.includes("s3cret"), out);
});
