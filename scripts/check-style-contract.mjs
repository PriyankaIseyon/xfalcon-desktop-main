import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const root = process.cwd();
// All of `src`, not a list of feature folders: `App.tsx`, `main.tsx` and
// `lib/` render production markup too, and an enumerated list silently stops
// covering whatever folder is added next.
const sourceRoots = ["src"];
const forbiddenUtilities = [
  "border-border",
  "text-muted-foreground",
  "text-primary",
  // A hand-rolled scrim. `--xf-ink` is near-white in the dark theme, so this
  // brightened the page it was meant to dim. `DialogShell` owns the scrim.
  "bg-foreground/30",
  // A control must name the properties it moves. `transition-all` can animate
  // layout or paint properties added later and can override a nested color
  // transition with a different contract.
  "transition-all",
];

// A literal colour answers no theme. White reads on the light accent and
// disappears on the dark one; the ink that is legible on a surface is always a
// token.
// One control focus treatment, and it is the outline `index.css` declares. A
// ring here is a second answer to the same question, and the primitives that
// drew one had drifted onto three different colours. Text FIELDS are the other
// focus role and keep their halo -- `focus:ring-*`, without `-visible`.
const forbiddenFocusPattern = {
  pattern: /(?<![\w-])focus-visible:(?:ring|outline-none)[\w-]*/g,
  use: "the control ring in index.css, which every focusable element already gets",
};

// P21-P27 have one feedback family. These names belonged to a second toast,
// a feature-local settings notice, and a second skeleton recipe. A source
// guard keeps a later caller from reviving any deleted path.
const forbiddenLegacyFeedbackPattern = {
  pattern: /(?<![\w-])(?:app-toast|ghost-block|settings-feedback)(?![\w-])/g,
  use: "Alert, Toaster, or Skeleton from src/ui",
};

const forbiddenLiteralColorPattern = {
  pattern: /(?<![\w-])(?:bg|text|border)-(?:white|black)(?:\/\d+)?(?![\w-])/g,
  use: "a token (bg-card, text-foreground, bg-accent-ink, …)",
};

// Type sizes live on the named ramp. An arbitrary `text-[13px]` and Tailwind's
// own fixed `text-base`/`text-lg` both sit off it: neither answers the
// reading-size setting, so they drift as the surface around them scales.
// Depth lives on the named elevations. Tailwind's own `shadow-sm`/`shadow-lg`
// are fixed light-theme rgba values: they do not read `--xf-elev-*`, which the
// dark theme redefines heavier precisely because a light-theme shadow is
// invisible there.
const forbiddenDepthPattern = {
  pattern: /(?<![\w-])shadow-(?:2xs|xs|sm|md|lg|xl|2xl)(?![\w-])/g,
  use: "shadow-e1, shadow-e2, or shadow-e3",
};

const forbiddenSizePatterns = [
  { pattern: /text-\[[0-9.]+(?:px|rem|em)\]/g, use: "a type role class (type-body, type-caption, …)" },
  { pattern: /(?<![\w-])text-(?:base|lg|xl|[2-9]xl)(?![\w-])/g, use: "type-heading, type-title, or type-display" },
];

// Only whole-line comments are stripped: enough to keep prose that *names* a
// forbidden utility from tripping the guard, without the quote tracking a real
// parser would need to strip trailing ones safely.
function withoutComments(source) {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^[ \t]*\/\/.*$/gm, "");
}

function productionTsxFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return productionTsxFiles(path);
    if (!entry.name.endsWith(".tsx") || entry.name.endsWith(".test.tsx")) return [];
    return [path];
  });
}

const invalidCallers = [];
for (const sourceRoot of sourceRoots) {
  for (const file of productionTsxFiles(join(root, sourceRoot))) {
    const source = withoutComments(readFileSync(file, "utf8"));
    for (const utility of forbiddenUtilities) {
      if (source.includes(utility)) invalidCallers.push(`${relative(root, file)}: ${utility}`);
    }
    for (const { pattern, use } of [
      ...forbiddenSizePatterns,
      forbiddenDepthPattern,
      forbiddenLiteralColorPattern,
      forbiddenFocusPattern,
      forbiddenLegacyFeedbackPattern,
    ]) {
      for (const match of source.match(pattern) ?? []) {
        invalidCallers.push(`${relative(root, file)}: ${match} — use ${use}`);
      }
    }
  }
}

if (invalidCallers.length > 0) {
  console.error("Style contract check failed: unsupported semantic utilities remain:");
  for (const caller of invalidCallers) console.error(`- ${caller}`);
  process.exit(1);
}

const assetsDir = join(root, "dist", "assets");
if (!statSync(assetsDir).isDirectory()) {
  console.error("Style contract check failed: dist/assets is missing. Run it after vite build.");
  process.exit(1);
}

// The primitive gallery is a developer tool. It is served by the dev server so
// the nine-mode sweep can drive the real components, and it is absent from
// `build.rollupOptions.input` so a production build never emits it — adding it
// there would put a second, unreachable surface in the bundle a user downloads.
for (const devGallery of ["primitives", "startup-visual"]) {
  if (readdirSync(join(root, "dist")).some((name) => name.startsWith(devGallery))) {
    console.error(`Style contract check failed: the ${devGallery} gallery reached dist/.`);
    console.error("  Remove it from build.rollupOptions.input in vite.config.ts.");
    process.exit(1);
  }
}

const cssFiles = readdirSync(assetsDir).filter((name) => name.endsWith(".css"));
if (cssFiles.length === 0) {
  console.error("Style contract check failed: vite produced no CSS asset.");
  process.exit(1);
}

const css = cssFiles.map((name) => readFileSync(join(assetsDir, name), "utf8")).join("\n");

// Every hand-written class a component wears has to exist in the sheet.
//
// `xf-switch` shipped applied to the switch and defined nowhere, so the class
// that was supposed to keep on and off distinguishable in forced-colors mode
// did nothing and nothing said so; two `app-sidebar__section--*` modifiers
// styled nothing at all. Tailwind utilities are generated from the markup and
// cannot go missing this way, so this reads only the two shapes the repo's own
// stylesheet uses: the `xf-` family, and BEM elements. Anything else is a
// utility and is not this rule's business.
// A heading states a role, never a size.
//
// `ProjectFileViewer` wore `text-sm font-semibold text-foreground` and `Modal`
// and `AlertDialog` each hand-wrote `type-heading font-semibold text-foreground`
// -- two copies of `SectionTitle`, written because the role was welded to an
// `<h3>` and a dialog title needs an `<h2>`. The roles take a `level` now, so
// there is nothing left for a caller to work around: a heading uses a role
// component, or a named class, and never a fixed Tailwind size that ignores the
// reading-size setting.
const FIXED_TEXT_SIZE = /(?<![\w-])text-(?:xs|sm|base|lg|xl|[2-9]xl)(?![\w-])/;
const offRampHeadings = [];
for (const sourceRoot of sourceRoots) {
  for (const file of productionTsxFiles(join(root, sourceRoot))) {
    const source = withoutComments(readFileSync(file, "utf8"));
    for (const match of source.matchAll(/<h([1-6])\s+[^>]*className\s*=\s*(?:"([^"]*)"|\{([\s\S]*?)\})/g)) {
      const classes = match[2] ?? match[3] ?? "";
      if (!FIXED_TEXT_SIZE.test(classes)) continue;
      const line = source.slice(0, match.index).split("\n").length;
      offRampHeadings.push(`${relative(root, file)}:${line}  <h${match[1]}> carries a fixed text size`);
    }
  }
}
if (offRampHeadings.length > 0) {
  console.error("Style contract check failed: headings must state a role, not a size:");
  for (const heading of offRampHeadings) console.error(`- ${heading}`);
  console.error("  Use HeroTitle/PageTitle/SectionTitle/SectionLabel/Eyebrow from ui/Heading, with `level`.");
  process.exit(1);
}

const HAND_WRITTEN_CLASS = /^(?:xf-[a-z0-9-]+|[a-z0-9-]+__[a-z0-9-]+(?:--[a-z0-9-]+)?)$/;
const undefinedClasses = new Map();
for (const sourceRoot of sourceRoots) {
  for (const file of productionTsxFiles(join(root, sourceRoot))) {
    const source = withoutComments(readFileSync(file, "utf8"));
    // `className="…"` and the string literals inside `className={…}`, rather
    // than every string in the file: `xf-preview` is a URI scheme, not a class.
    for (const attribute of source.matchAll(/className\s*=\s*(?:"([^"]*)"|\{([\s\S]*?)\})/g)) {
      const literals =
        attribute[1] != null
          ? [attribute[1]]
          : [...(attribute[2] ?? "").matchAll(/"([^"]*)"|'([^']*)'|`([^`$]*)`/g)].map((m) => m[1] ?? m[2] ?? m[3]);
      for (const literal of literals) {
        for (const token of literal.split(/\s+/)) {
          if (!HAND_WRITTEN_CLASS.test(token)) continue;
          if (new RegExp(`\\.${token}(?![\\w-])`).test(css)) continue;
          if (!undefinedClasses.has(token)) undefinedClasses.set(token, new Set());
          undefinedClasses.get(token).add(relative(root, file));
        }
      }
    }
  }
}
if (undefinedClasses.size > 0) {
  console.error("Style contract check failed: classes are applied but never defined:");
  for (const [token, files] of undefinedClasses) {
    console.error(`- .${token} — worn by ${[...files].join(", ")}, absent from the built sheet`);
  }
  process.exit(1);
}
for (const selector of [
  /\.app-toast(?:\[|,|\{|\s)/,
  /\.ghost-block(?:\[|,|\{|\s)/,
  /\.settings-feedback(?:[-_]|\[|,|\{|\s)/,
]) {
  if (selector.test(css)) {
    console.error(`Style contract check failed: built CSS contains obsolete feedback selector ${selector}.`);
    process.exit(1);
  }
}

for (const selector of [
  ".border-default",
  ".text-muted",
  ".text-foreground",
  ".type-display",
  ".type-title",
  ".type-heading",
  ".type-body",
  ".type-label",
  ".type-caption",
  ".type-micro",
  ".dialog-shell",
  ".dialog-scrim",
  ".surface-card",
  ".xf-switch",
  ".app-toaster__toast",
  ".skeleton-shimmer",
  ".xf-progress-indeterminate",
]) {
  if (!css.includes(selector)) {
    console.error(`Style contract check failed: built CSS is missing ${selector}.`);
    process.exit(1);
  }
}

for (const token of [
  "--xf-control-compact:",
  "--xf-control-default:",
  "--xf-control-prominent:",
  "--xf-hit-target-coarse:",
  "--xf-icon-compact:",
  "--xf-icon-default:",
  "--xf-icon-prominent:",
  "--xf-icon-stroke-regular:",
  "--xf-icon-stroke-emphasis:",
  // Ink ON danger. Without it the destructive button paints its label with a
  // surface token, which contrasts by luck rather than by role.
  "--xf-danger-ink:",
  "--xf-type-display:",
  "--xf-type-title:",
  "--xf-type-heading:",
  "--xf-type-reading:",
  "--xf-type-body:",
  "--xf-type-label:",
  "--xf-type-caption:",
  "--xf-type-micro:",
  // The chart resolves these at render time and writes the result into the SVG
  // as literal presentation attributes, so a missing token would not throw --
  // it would quietly draw the light palette on a dark canvas.
  "--xf-chart-1:",
  "--xf-chart-2:",
  "--xf-chart-3:",
  "--xf-chart-4:",
  "--xf-chart-5:",
  "--xf-chart-6:",
  "--xf-chart-7:",
  "--xf-chart-axis:",
  "--xf-chart-grid:",
  "--xf-chart-surface:",
]) {
  if (!css.includes(token)) {
    console.error(`Style contract check failed: built CSS is missing ${token}.`);
    process.exit(1);
  }
}

// A token that exists only at `:root` answers no theme. The dark block has to
// redefine every series colour; the chrome tokens follow `--xf-ink-3` and
// `--xf-border`, which the dark block already sets.
// The BASE dark block, identified by the surface it sets -- not any block that
// happens to match `[data-theme=dark]`. `prefers-contrast: more` opens a second
// one, and joining the two would let the contrast override stand in for a
// palette the ordinary dark theme never defines.
const darkBlock = /\[data-theme=(?:"dark"|dark)\]\{([^}]*)\}/g;
const darkDeclarations =
  [...css.matchAll(darkBlock)].map((match) => match[1]).find((block) => block.includes("--xf-canvas:")) ?? "";
if (darkDeclarations === "") {
  console.error("Style contract check failed: built CSS has no base [data-theme=dark] palette.");
  process.exit(1);
}
for (let index = 1; index <= 7; index++) {
  if (!darkDeclarations.includes(`--xf-chart-${index}:`)) {
    console.error(`Style contract check failed: the dark theme does not redefine --xf-chart-${index}.`);
    process.exit(1);
  }
}
if (!darkDeclarations.includes("--xf-danger-ink:")) {
  console.error("Style contract check failed: the dark theme does not redefine --xf-danger-ink.");
  process.exit(1);
}

// No visited-link colour, anywhere. This app renders links from model output
// and from remote content, and a `:visited` style is a side channel that
// reports a person's browsing history to whoever authored the page. Tailwind's
// preflight already leaves anchors on the inherited colour; this keeps a
// stylesheet nobody reads from quietly reintroducing the distinction.
const visited = css.match(/[^{}]*:visited[^{}]*\{/g) ?? [];
if (visited.length > 0) {
  console.error("Style contract check failed: the built CSS styles :visited links:");
  for (const selector of visited) console.error(`- ${selector.replace(/\{$/, "").trim()}`);
  process.exit(1);
}

for (const declaration of [
  "transition-property:transform,color,background-color,border-color,box-shadow",
  "transition-duration:var(--xf-dur-fast)",
  "transition-timing-function:var(--xf-ease-soft)",
  ".press:active{transform:scale(.98)}",
]) {
  if (!css.includes(declaration)) {
    console.error(`Style contract check failed: built CSS is missing ${declaration}.`);
    process.exit(1);
  }
}

console.log("Style contract check passed.");
