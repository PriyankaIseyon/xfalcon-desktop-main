/**
 * How the project file view handles a file by size (spec: project-file-editing,
 * "A local file opens at any size up to 10 GiB").
 *
 * One table, because the tiers have to agree with each other: a file that loads
 * whole must be one the rich view may render, and the editor must never be
 * offered a file the view did not load whole. None of these bound what reaches
 * a model or a server — the bytes stay on this device.
 */

const MIB = 1024 * 1024;

export const PROJECT_FILE_LIMITS = {
  /** Past this the view shows the file's size and reads nothing. Mirrors
   * `MAX_VIEWABLE_FILE_BYTES` in `src-tauri/src/projects/viewer.rs`. */
  viewableBytes: 10 * 1024 * MIB,
  /** Loaded whole and held as one string. A JavaScript string holds about
   * 536 million characters; this stays far inside that, and inside what a
   * virtualized list scrolls without effort (about a million lines). */
  wholeLoadBytes: 64 * MIB,
  /** One window of the paged view past `wholeLoadBytes`. */
  windowBytes: 1 * MIB,
  /** Highlighted in one pass. highlight.js runs on the main thread; past this
   * the pass is long enough to feel, and the lines show plain. */
  highlightBytes: 2 * MIB,
  /** Rendered as Markdown prose through the chat's pipeline. */
  markdownBytes: 4 * MIB,
  /** Parsed into the chat's table in the webview. Past this the backend pages
   * the rows instead. */
  inMemoryTableBytes: 16 * MIB,
  /** Previewed from disk. Mirrors `MAX_PROJECT_PREVIEW_BYTES`. */
  previewBytes: 32 * MIB,
  /** Offered for editing in the text editor. */
  editBytes: 16 * MIB,
} as const;
