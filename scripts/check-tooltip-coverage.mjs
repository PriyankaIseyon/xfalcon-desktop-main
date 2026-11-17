#!/usr/bin/env node
/**
 * The product has one hover-and-focus help contract: `src/ui/Tooltip.tsx`.
 *
 * A native `title` is a second grammar for the same job, and a worse one -- it
 * cannot be styled, it never reaches a keyboard user, its delay is the
 * operating system's rather than the product's, and it is invisible to the
 * visual suite. This guard keeps it out.
 *
 * Three things are checked:
 *
 *   1. No `title` attribute on a DOM element, except `<iframe>`, where `title`
 *      is the accessible name the platform requires (WCAG 4.1.2).
 *   2. No `title` prop on a primitive that spreads the rest onto a DOM node,
 *      which is the same attribute wearing a component's name.
 *   3. No imperative `.title =` outside `TruncatedText`, the one owner of
 *      truncation reveal -- text that is clipped repeats itself on hover, which
 *      is not a description and must not take focus.
 *
 * It also reports icon-action coverage: every `IconButton` must carry an
 * accessible name, because that name is what the tooltip shows by default.
 */
import { existsSync, readFileSync } from "node:fs";
import { execSync } from "node:child_process";
import path from "node:path";

const repoRoot = path.resolve(import.meta.dirname, "..");

/** `title` is the accessible name of an embedded document, not a hint. */
const NATIVE_TITLE_ALLOWED = new Set(["iframe"]);

/** Primitives that spread their remaining props onto a DOM node. */
const SPREADING_PRIMITIVES = [
  "Avatar",
  "Badge",
  "Button",
  "Card",
  "Chip",
  "IconButton",
  "Kbd",
  "Link",
  "Panel",
  "SettingsButton",
  "SettingsIconButton",
];

/** Named because the tooltip they show is their accessible name. */
const ICON_ACTIONS = ["IconButton", "SettingsIconButton"];

/** The single owner of truncation reveal. */
const TITLE_ASSIGNMENT_OWNER = "src/ui/TruncatedText.tsx";

/**
 * Walks the attribute span of one JSX opening tag.
 *
 * Comments are tracked as their own state: a `// ... mouse's affordance` line
 * inside a tag would otherwise open a string that never closes, and the scan
 * would run past the tag and report a `title` that belongs to something else.
 */
function attributeSpan(source, from) {
  let depth = 0;
  let quote = null;
  let comment = null;
  let attrs = "";
  for (let i = from; i < source.length; i++) {
    const c = source[i];
    const next = source[i + 1];
    if (comment === "line") {
      if (c === "\n") comment = null;
    } else if (comment === "block") {
      if (c === "*" && next === "/") {
        comment = null;
        i++;
      }
    } else if (quote) {
      if (c === quote && source[i - 1] !== "\\") quote = null;
      attrs += c;
    } else if (c === "/" && next === "/") {
      comment = "line";
    } else if (c === "/" && next === "*") {
      comment = "block";
    } else if (c === '"' || c === "'" || c === "`") {
      quote = c;
      attrs += c;
    } else if (c === "{") {
      depth++;
      attrs += c;
    } else if (c === "}") {
      depth--;
      attrs += c;
    } else if (c === ">" && depth === 0) {
      return attrs;
    } else {
      attrs += c;
    }
  }
  return null;
}

function* elements(source, name) {
  const opener = new RegExp(`<(${name})[\\s>]`, "g");
  let match;
  while ((match = opener.exec(source)) != null) {
    const attrs = attributeSpan(source, opener.lastIndex - 1);
    if (attrs == null) continue;
    yield { tag: match[1], attrs, line: source.slice(0, match.index).split("\n").length };
  }
}

const hasAttribute = (attrs, name) => new RegExp(`(^|\\s)${name}\\s*=`).test(attrs);
const spreadsProps = (attrs) => /\{\s*\.\.\./.test(attrs);

// `git ls-files` reads the INDEX, and the index keeps a file that has been
// deleted in the working tree until the deletion is staged. The guard checks
// source that exists, so a tracked-but-deleted path is skipped rather than
// read -- otherwise every mid-batch deletion crashes the build with an ENOENT
// stack instead of running the check.
const files = execSync("git ls-files 'src/**/*.tsx'", { cwd: repoRoot, encoding: "utf8" })
  .split("\n")
  .filter(Boolean)
  .filter((file) => existsSync(path.join(repoRoot, file)));

const failures = [];
let iconActions = 0;

for (const file of files) {
  const source = readFileSync(path.join(repoRoot, file), "utf8");

  for (const element of elements(source, "[a-z][a-zA-Z0-9-]*")) {
    if (!hasAttribute(element.attrs, "title")) continue;
    if (NATIVE_TITLE_ALLOWED.has(element.tag)) continue;
    failures.push(
      `${file}:${element.line}  <${element.tag} title=…>  ` +
        `use <Tooltip label=…> for a hint, or <TruncatedText> for clipped text`,
    );
  }

  for (const element of elements(source, SPREADING_PRIMITIVES.join("|"))) {
    if (hasAttribute(element.attrs, "title")) {
      failures.push(
        `${file}:${element.line}  <${element.tag} title=…>  ` +
          `this lands on the DOM node -- use the tooltip prop, or <Tooltip>`,
      );
    }
  }

  // Coverage is a claim about the product, so a test fixture neither counts
  // toward it nor fails it.
  const isTest = file.endsWith(".test.tsx");
  for (const element of isTest ? [] : elements(source, ICON_ACTIONS.join("|"))) {
    iconActions++;
    if (spreadsProps(element.attrs)) continue;
    if (hasAttribute(element.attrs, "aria-label") || hasAttribute(element.attrs, "aria-labelledby")) continue;
    failures.push(
      `${file}:${element.line}  <${element.tag}>  ` +
        `an icon action needs aria-label -- it is the accessible name and the tooltip`,
    );
  }

  if (file !== TITLE_ASSIGNMENT_OWNER) {
    const assignment = /(^|[^\w.])(\w+)\.title\s*=[^=]/m.exec(source);
    if (assignment) {
      const line = source.slice(0, assignment.index).split("\n").length;
      failures.push(`${file}:${line}  ${assignment[2]}.title = …  only ${TITLE_ASSIGNMENT_OWNER} sets a native title`);
    }
  }
}

if (failures.length > 0) {
  console.error("Tooltip contract check failed:\n");
  for (const failure of failures) console.error(`  ${failure}`);
  console.error(`\n${failures.length} violation(s).`);
  process.exit(1);
}

console.log(`Tooltip contract check passed. ${iconActions} icon actions, all named.`);
