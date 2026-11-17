/** Regression guards (spec: app-typography): the product type contract is
 * pinned at the source level — token stacks, face files, preloads. Any of
 * these drifting is how "the app is not in the product font" ships. */
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { reportFontActivation } from "./typography";

// Vitest executes on node. The import is dynamic because the app project does
// not carry node's globals — `@types/node` types the module itself, which is all
// this needs. (`tokens.css?raw` is NOT usable here: vitest's css pipeline stubs
// raw css imports to an empty string.)
type ReadFile = (path: URL, encoding: "utf8") => string;
type Exists = (path: URL) => boolean;
let readFileSync: ReadFile;
let existsSync: Exists;
beforeAll(async () => {
  const fs = await import("node:fs");
  readFileSync = fs.readFileSync as ReadFile;
  existsSync = fs.existsSync as Exists;
});

const at = (relative: string) => new URL(relative, import.meta.url);

describe("product type stacks", () => {
  it("are the product faces ALONE — no silent lookalike fallbacks", () => {
    const tokens = readFileSync(at("../styles/tokens.css"), "utf8");
    // Owner directive 2026-08-06: a missing face must fail LOUD (browser
    // last-resort serif), never blend into Segoe/system-ui.
    expect(tokens).toMatch(/--xf-font-sans:\s*"Google Sans";/);
    expect(tokens).toMatch(/--xf-font-mono:\s*"Geist Mono";/);
  });

  it("declare @font-face sources that exist in public/fonts", () => {
    const tokens = readFileSync(at("../styles/tokens.css"), "utf8");
    const sources = [...tokens.matchAll(/url\("\/fonts\/([^"]+)"\)/g)].map((match) => match[1]);
    expect(sources.length).toBeGreaterThanOrEqual(3);
    for (const file of sources) {
      expect(existsSync(at(`../../public/fonts/${file}`)), `public/fonts/${file}`).toBe(true);
    }
  });
});

describe("index.html", () => {
  it("preloads the primary sans and mono faces", () => {
    const indexHtml = readFileSync(at("../../index.html"), "utf8");
    for (const [file, type] of [
      ["GoogleSans-Variable.ttf", "font/ttf"],
      ["GeistMono.woff2", "font/woff2"],
    ]) {
      const preload = new RegExp(`<link rel="preload" href="/fonts/${file}" as="font" type="${type}" crossorigin />`);
      expect(indexHtml).toMatch(preload);
    }
  });
});

describe("reportFontActivation", () => {
  afterEach(() => vi.restoreAllMocks());

  function stubFonts(check: (query: string) => boolean) {
    Object.defineProperty(document, "fonts", {
      configurable: true,
      value: { ready: Promise.resolve(), check },
    });
  }

  it("logs one structured error per missing family", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    stubFonts(() => false);
    reportFontActivation();
    await Promise.resolve();
    await Promise.resolve();
    expect(error).toHaveBeenCalledWith("font_activation_failed", { family: "Google Sans" });
    expect(error).toHaveBeenCalledWith("font_activation_failed", { family: "Geist Mono" });
    expect(error).toHaveBeenCalledTimes(2);
  });

  it("stays silent when the faces activate", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    stubFonts(() => true);
    reportFontActivation();
    await Promise.resolve();
    await Promise.resolve();
    expect(error).not.toHaveBeenCalled();
  });
});
