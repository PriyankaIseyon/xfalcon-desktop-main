/**
 * A filesystem path as a person should read it.
 *
 * Windows canonicalisation (which is what `std::fs::canonicalize` on the Rust
 * side returns, and what a trusted project's `root_path` therefore holds)
 * produces extended-length paths: `\\?\D:\work\project`. That prefix is real
 * and load-bearing — it is what lifts the 260-character MAX_PATH limit and
 * turns off path parsing — so it must stay on the value used for I/O. It just
 * has no business being shown to anyone.
 *
 * Display-only, deliberately. Stripping the prefix at the boundary where the
 * path is *stored* would silently re-impose MAX_PATH on deep project trees;
 * stripping it where the path is *printed* costs nothing and cannot.
 *
 * - `\\?\D:\work\project`      → `D:\work\project`
 * - `\\?\UNC\server\share\dir` → `\\server\share\dir`
 * - anything else              → unchanged
 */
export function displayPath(path: string): string {
  if (path.startsWith("\\\\?\\UNC\\")) return `\\\\${path.slice("\\\\?\\UNC\\".length)}`;
  if (path.startsWith("\\\\?\\")) return path.slice("\\\\?\\".length);
  return path;
}
