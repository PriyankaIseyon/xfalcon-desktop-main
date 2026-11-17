import assert from "node:assert/strict";
import { once } from "node:events";
import { createServer } from "node:http";
import test from "node:test";

import { cleanupDesktopReleases } from "./cleanup-desktop-releases.mjs";

const release = (id) => ({
  databaseId: id,
  tagName: `v0.5.${id}`,
  createdAt: new Date(Date.UTC(2026, 0, 1) + id * 1000).toISOString(),
  isLatest: false,
});

async function fixture(t, releases, { failPage, listBody, deleteStatus = 204, redirect = false } = {}) {
  const requests = [];
  const server = createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : null;
    requests.push({ method: request.method, path: request.url, body });
    assert.equal(request.headers.authorization, "Bearer fixture-token");
    if (request.method === "POST" && request.url === "/graphql") {
      assert.equal(body.variables.owner, "fixture");
      assert.equal(body.variables.name, "desktop");
      assert.match(body.query, /first: 100/);
      assert.match(body.query, /field: CREATED_AT, direction: DESC/);
      if (redirect) {
        response.writeHead(302, { Location: "/redirect-target" });
        response.end();
        return;
      }
      const offset = Number(body.variables.cursor ?? 0);
      if (failPage === offset) {
        response.writeHead(403);
        response.end();
        return;
      }
      const nodes = releases.slice(offset, offset + 100);
      const payload = {
        data: {
          repository: {
            releases: {
              nodes,
              pageInfo: {
                hasNextPage: offset + nodes.length < releases.length,
                endCursor: String(offset + nodes.length),
              },
            },
          },
        },
      };
      response.writeHead(200, { "Content-Type": "application/json" });
      response.end(JSON.stringify(listBody ? listBody(payload) : payload));
      return;
    }
    if (request.method === "DELETE" && /^\/repos\/fixture\/desktop\/releases\/\d+$/.test(request.url)) {
      response.writeHead(deleteStatus);
      response.end();
      return;
    }
    response.writeHead(404);
    response.end();
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => new Promise((resolve) => server.close(resolve)));
  const base = `http://127.0.0.1:${server.address().port}`;
  return {
    requests,
    options: {
      repository: "fixture/desktop",
      releaseTag: releases.findLast((entry) => entry?.tagName)?.tagName,
      token: "fixture-token",
      fetchImpl: (url, init) => {
        const target = new URL(url);
        assert.equal(target.origin, "https://api.github.com");
        return fetch(`${base}${target.pathname}${target.search}`, init);
      },
    },
  };
}

test("keeps five recent releases and deletes older entries through the real HTTP consumer", async (t) => {
  const { requests, options } = await fixture(
    t,
    Array.from({ length: 8 }, (_, i) => release(i + 1)),
  );
  const result = await cleanupDesktopReleases(options);
  assert.deepEqual(result.kept, ["v0.5.8", "v0.5.7", "v0.5.6", "v0.5.5", "v0.5.4"]);
  assert.deepEqual(result.removed, ["v0.5.3", "v0.5.2", "v0.5.1"]);
  assert.deepEqual(
    requests.filter((request) => request.method === "DELETE").map((request) => request.path),
    ["/repos/fixture/desktop/releases/3", "/repos/fixture/desktop/releases/2", "/repos/fixture/desktop/releases/1"],
  );
});

for (const count of [3, 5]) {
  test(`does not delete when only ${count} releases exist`, async (t) => {
    const { requests, options } = await fixture(
      t,
      Array.from({ length: count }, (_, i) => release(i + 1)),
    );
    const result = await cleanupDesktopReleases(options);
    assert.equal(result.kept.length, count);
    assert.deepEqual(result.removed, []);
    assert.equal(requests.length, 1);
  });
}

test("includes every page before deleting and uses release age rather than tag text", async (t) => {
  const { requests, options } = await fixture(
    t,
    Array.from({ length: 103 }, (_, i) => release(i + 1)),
  );
  const result = await cleanupDesktopReleases(options);
  assert.deepEqual(result.kept, ["v0.5.103", "v0.5.102", "v0.5.101", "v0.5.100", "v0.5.99"]);
  assert.equal(result.removed.length, 98);
  assert.equal(requests[0].body.variables.cursor, null);
  assert.equal(requests[1].body.variables.cursor, "100");
  assert.equal(requests.slice(2).filter((request) => request.method === "DELETE").length, 98);
});

test("preserves the active published release within the five slots", async (t) => {
  const releases = Array.from({ length: 8 }, (_, i) => release(i + 1));
  releases[0].isLatest = true;
  const { requests, options } = await fixture(t, releases);
  const result = await cleanupDesktopReleases(options);
  assert.deepEqual(result.kept, ["v0.5.8", "v0.5.7", "v0.5.6", "v0.5.5", "v0.5.1"]);
  assert.deepEqual(result.removed, ["v0.5.4", "v0.5.3", "v0.5.2"]);
  assert.ok(!requests.some((request) => request.method === "DELETE" && request.path.endsWith("/1")));
});

test("dry-run reads real listing data without sending a delete", async (t) => {
  const { requests, options } = await fixture(
    t,
    Array.from({ length: 8 }, (_, i) => release(i + 1)),
  );
  const result = await cleanupDesktopReleases({ ...options, dryRun: true });
  assert.equal(result.dryRun, true);
  assert.equal(result.removed.length, 3);
  assert.equal(requests.length, 1);
});

for (const tag of ["v0.5.99", "v0.5.1"]) {
  test(`refuses cleanup when the new release ${tag} is absent from the retained set`, async (t) => {
    const { requests, options } = await fixture(
      t,
      Array.from({ length: 8 }, (_, i) => release(i + 1)),
    );
    await assert.rejects(cleanupDesktopReleases({ ...options, releaseTag: tag }), /cleanup refused/);
    assert.equal(requests.length, 1);
  });
}

test("a later listing-page failure deletes nothing", async (t) => {
  const { requests, options } = await fixture(
    t,
    Array.from({ length: 103 }, (_, i) => release(i + 1)),
    {
      failPage: 100,
    },
  );
  await assert.rejects(cleanupDesktopReleases(options), /listing failed: HTTP 403/);
  assert.equal(requests.length, 2);
  assert.ok(requests.every((request) => request.method === "POST"));
});

test("GraphQL partial failures delete nothing", async (t) => {
  const { requests, options } = await fixture(t, [release(1)], {
    listBody: (payload) => ({ ...payload, errors: [{ message: "permission failure" }] }),
  });
  await assert.rejects(cleanupDesktopReleases(options), /complete release listing/);
  assert.equal(requests.length, 1);
});

for (const invalid of [null, { databaseId: "1/../other" }, { createdAt: "invalid" }, { isLatest: null }]) {
  test(`invalid external release metadata deletes nothing: ${JSON.stringify(invalid)}`, async (t) => {
    const releases = Array.from({ length: 8 }, (_, i) => release(i + 1));
    releases[0] = invalid === null ? null : { ...releases[0], ...invalid };
    const { requests, options } = await fixture(t, releases);
    await assert.rejects(cleanupDesktopReleases(options), /invalid release metadata/);
    assert.equal(requests.length, 1);
  });
}

test("duplicate release IDs delete nothing", async (t) => {
  const releases = Array.from({ length: 8 }, (_, i) => release(i + 1));
  releases[0].databaseId = 2;
  const { requests, options } = await fixture(t, releases);
  await assert.rejects(cleanupDesktopReleases(options), /invalid release metadata/);
  assert.equal(requests.length, 1);
});

test("a repeated pagination cursor deletes nothing", async (t) => {
  const { requests, options } = await fixture(
    t,
    Array.from({ length: 103 }, (_, i) => release(i + 1)),
    {
      listBody: (payload) => {
        payload.data.repository.releases.pageInfo = { hasNextPage: true, endCursor: "100" };
        return payload;
      },
    },
  );
  await assert.rejects(cleanupDesktopReleases(options), /invalid release cursor/);
  assert.equal(requests.length, 2);
});

test("a deletion failure stops cleanup and is reported", async (t) => {
  const { requests, options } = await fixture(
    t,
    Array.from({ length: 8 }, (_, i) => release(i + 1)),
    {
      deleteStatus: 403,
    },
  );
  await assert.rejects(cleanupDesktopReleases(options), /v0\.5\.3 cleanup failed: HTTP 403/);
  assert.equal(requests.filter((request) => request.method === "DELETE").length, 1);
});

test("API redirects are rejected instead of forwarding the token", async (t) => {
  const { requests, options } = await fixture(t, [release(1)], { redirect: true });
  await assert.rejects(cleanupDesktopReleases(options), /fetch failed/);
  assert.deepEqual(
    requests.map((request) => request.path),
    ["/graphql"],
  );
});
