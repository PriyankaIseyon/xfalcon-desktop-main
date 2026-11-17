import { resolveLanguage } from "../chat/richContent/codeLanguages";

/**
 * How the project file view renders a file, chosen from its name (spec:
 * project-file-editing, "A file renders the way the chat renders its type").
 *
 * Every kind but `plain` is a rendering the chat transcript already has; this
 * module only decides which one a file name asks for. Content is never sniffed
 * here: a `.csv` that fails to parse falls back to plain text in the view,
 * rather than this guessing a type the name did not claim.
 */
export type ProjectFileRendering =
  | { kind: "html" }
  | { kind: "svg" }
  | { kind: "markdown" }
  | { kind: "delimited"; delimiter: "," | "\t" }
  /** A table when the file is an array of objects, highlighted JSON otherwise. */
  | { kind: "json" }
  /** A table when every non-blank line is an object. */
  | { kind: "jsonl" }
  /** `tag` is a tag the chat's language registry knows. */
  | { kind: "code"; tag: string }
  | { kind: "plain" };

const PLAIN: ProjectFileRendering = { kind: "plain" };

const BY_EXTENSION: Readonly<Record<string, ProjectFileRendering>> = {
  html: { kind: "html" },
  htm: { kind: "html" },
  xhtml: { kind: "html" },
  svg: { kind: "svg" },
  md: { kind: "markdown" },
  markdown: { kind: "markdown" },
  csv: { kind: "delimited", delimiter: "," },
  tsv: { kind: "delimited", delimiter: "\t" },
  tab: { kind: "delimited", delimiter: "\t" },
  json: { kind: "json" },
  jsonl: { kind: "jsonl" },
  ndjson: { kind: "jsonl" },
  // Plain on purpose. Text and logs gain nothing from a card, and `m` is a
  // registry alias of Power Query while a `.m` file is far more often MATLAB
  // or Objective-C — a confidently wrong grammar is worse than none.
  txt: PLAIN,
  text: PLAIN,
  log: PLAIN,
  m: PLAIN,
};

/** Extensions that name a registry language by a different word. */
const EXTENSION_TAG: Readonly<Record<string, string>> = {
  jl: "julia",
  kts: "kotlin",
  cxx: "cpp",
  hxx: "cpp",
  hh: "cpp",
  pyi: "python",
  psm1: "powershell",
  hql: "hiveql",
  ipynb: "json",
};

/** File names that carry their language without an extension. */
const BY_NAME: Readonly<Record<string, ProjectFileRendering>> = {
  dockerfile: { kind: "code", tag: "dockerfile" },
  containerfile: { kind: "code", tag: "dockerfile" },
  makefile: { kind: "code", tag: "makefile" },
  gnumakefile: { kind: "code", tag: "makefile" },
};

/** The rendering for a project-relative `path`, compared without regard to case. */
export function projectFileRendering(path: string): ProjectFileRendering {
  const name = (path.split("/").pop() ?? "").toLowerCase();
  const named = BY_NAME[name];
  if (named !== undefined) return named;
  const dot = name.lastIndexOf(".");
  // No extension, or a dotfile such as `.gitignore`, whose "extension" is its name.
  if (dot <= 0 || dot === name.length - 1) return PLAIN;
  const extension = name.slice(dot + 1);
  const fixed = BY_EXTENSION[extension];
  if (fixed !== undefined) return fixed;
  const tag = EXTENSION_TAG[extension] ?? extension;
  return resolveLanguage(tag).known ? { kind: "code", tag } : PLAIN;
}
