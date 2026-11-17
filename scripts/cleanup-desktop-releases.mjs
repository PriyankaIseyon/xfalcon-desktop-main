import { pathToFileURL } from "node:url";

// GraphQL createdAt is the release's creation time, rather than its tag commit's
// date. Include drafts because desktop-build publishes staging releases as drafts.
const QUERY = `query($owner: String!, $name: String!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    releases(first: 100, after: $cursor, orderBy: {field: CREATED_AT, direction: DESC}) {
      nodes { databaseId tagName createdAt isLatest }
      pageInfo { hasNextPage endCursor }
    }
  }
}`;

export async function cleanupDesktopReleases({ repository, releaseTag, token, dryRun = false, fetchImpl = fetch }) {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*\/[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(repository ?? ""))
    throw new Error("A valid GitHub repository is required");
  if (typeof token !== "string" || !token.trim()) throw new Error("GH_TOKEN is required");
  if (typeof releaseTag !== "string" || !releaseTag) throw new Error("The new release tag is required");

  const [owner, name] = repository.split("/");
  const headers = {
    Authorization: `Bearer ${token}`,
    Accept: "application/vnd.github+json",
    "Content-Type": "application/json",
    "X-GitHub-Api-Version": "2022-11-28",
  };
  const releases = [];
  const cursors = new Set();
  let cursor = null;
  for (;;) {
    const response = await fetchImpl("https://api.github.com/graphql", {
      method: "POST",
      headers,
      redirect: "error",
      signal: AbortSignal.timeout(30_000),
      body: JSON.stringify({ query: QUERY, variables: { owner, name, cursor } }),
    });
    if (!response.ok) throw new Error(`Release listing failed: HTTP ${response.status}`);
    const page = await response.json();
    const connection = page.data?.repository?.releases;
    if (page.errors?.length || !Array.isArray(connection?.nodes))
      throw new Error("GitHub did not return a complete release listing");
    releases.push(...connection.nodes);
    if (typeof connection.pageInfo?.hasNextPage !== "boolean")
      throw new Error("GitHub did not return release pagination metadata");
    if (!connection.pageInfo.hasNextPage) break;
    cursor = connection.pageInfo.endCursor;
    if (typeof cursor !== "string" || !cursor || cursors.has(cursor))
      throw new Error("GitHub returned an invalid release cursor");
    cursors.add(cursor);
  }

  const ids = new Set();
  for (const release of releases) {
    if (
      !Number.isSafeInteger(release?.databaseId) ||
      release.databaseId <= 0 ||
      ids.has(release.databaseId) ||
      typeof release.tagName !== "string" ||
      !release.tagName ||
      typeof release.createdAt !== "string" ||
      !Number.isFinite(Date.parse(release.createdAt)) ||
      typeof release.isLatest !== "boolean"
    )
      throw new Error("GitHub returned invalid release metadata");
    ids.add(release.databaseId);
  }
  releases.sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt) || b.databaseId - a.databaseId);
  const kept = releases.slice(0, 5);
  const latest = releases.filter((release) => release.isLatest);
  if (latest.length > 1) throw new Error("GitHub returned multiple latest releases");
  // Keep the release serving GitHub's latest-download endpoint within five slots.
  if (latest.length && !kept.includes(latest[0])) kept[kept.length - 1] = latest[0];
  if (!kept.some((release) => release.tagName === releaseTag))
    throw new Error("The new release is absent from the retained releases; cleanup refused");
  const removed = releases.filter((release) => !kept.includes(release));
  if (!dryRun) {
    for (const release of removed) {
      const response = await fetchImpl(`https://api.github.com/repos/${repository}/releases/${release.databaseId}`, {
        method: "DELETE",
        headers,
        redirect: "error",
        signal: AbortSignal.timeout(30_000),
      });
      if (response.status !== 204)
        throw new Error(`Release ${release.tagName} cleanup failed: HTTP ${response.status}`);
    }
  }
  return { dryRun, kept: kept.map((release) => release.tagName), removed: removed.map((release) => release.tagName) };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [releaseTag, option, ...extra] = process.argv.slice(2);
  if (!releaseTag || (option && option !== "--dry-run") || extra.length)
    throw new Error("Usage: cleanup-desktop-releases.mjs RELEASE_TAG [--dry-run]");
  const result = await cleanupDesktopReleases({
    repository: process.env.GITHUB_REPOSITORY,
    releaseTag,
    token: process.env.GH_TOKEN,
    dryRun: option === "--dry-run",
  });
  console.log(JSON.stringify(result, null, 2));
}
