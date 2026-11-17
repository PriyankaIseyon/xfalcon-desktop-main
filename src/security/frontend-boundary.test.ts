// @vitest-environment node
/// <reference types="vite/client" />

import react from "@vitejs/plugin-react";
import { describe, expect, it } from "vitest";
import tauriConfig from "../../src-tauri/tauri.conf.json";
import indexHtml from "../../index.html?raw";
import migrationHtml from "../../storage-migration.html?raw";

const sources = import.meta.glob("/src/**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

// Every exclusion here needs its own dedicated test below, narrowly scoped to
// exactly the one browser API it needs -- the exclusion list is not a bypass,
// it's a pointer to where the audit trail lives.
const AUDITED_EXCEPTIONS = [
  "/src/security/storage-migration.ts",
  "/src/features/chat/thinkingPhrase.ts",
  "/src/app/useIdleSignOut.ts",
];

function productionSources(): Array<[string, string]> {
  return Object.entries(sources).filter(
    ([path]) =>
      !path.includes("/test/") && !/\.(?:test|spec)\.[jt]sx?$/.test(path) && !AUDITED_EXCEPTIONS.includes(path),
  );
}

function source(path: string): string {
  const contents = sources[`/src/${path}`];
  if (contents === undefined) throw new Error(`Missing source: ${path}`);
  return contents;
}

describe("frontend privilege boundary", () => {
  it("pins the development CSP to the exact Vite React Refresh preamble", async () => {
    const preamble = react.preambleCode.replace("__BASE__", "/");
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(preamble));
    const hash = btoa(String.fromCharCode(...new Uint8Array(digest)));
    const developmentCsp = tauriConfig.app.security.devCsp;

    expect(developmentCsp).toContain(`'sha256-${hash}'`);
    expect(developmentCsp).not.toContain("'unsafe-inline'");
    expect(developmentCsp).not.toContain("'unsafe-eval'");
  });

  it("loads development CSS as a self-hosted stylesheet instead of an injected inline style", () => {
    expect(indexHtml).toContain('<link rel="stylesheet" href="/src/index.css?direct" />');
    expect(source("main.tsx")).not.toContain('import "./index.css"');
  });

  it("contains no browser network, storage, timer, or forbidden native authority", () => {
    const browserAuthority = [/\bfetch\s*\(/, /\blocalStorage\b/, /\bsessionStorage\b/, /\bsetInterval\s*\(/];
    const forbiddenCommands = [
      "get_file_info",
      "upload_file_chunk",
      "read_file_bytes",
      "read_text_file",
      "write_text_file",
      "list_dir_files",
      "run_shell",
      "fetch_url",
    ];
    const violations = productionSources().flatMap(([path, contents]) => {
      const matches = [
        ...browserAuthority.filter((pattern) => pattern.test(contents)).map(String),
        ...forbiddenCommands.filter((command) => contents.includes(command)),
      ];
      return matches.length > 0 ? [`${path}: ${matches.join(", ")}`] : [];
    });

    expect(violations).toEqual([]);
  });

  it("keeps the migration module as an audited browser-storage exception", () => {
    const migration = source("security/storage-migration.ts");
    expect(migration).toContain("localStorage");
    expect(migration).toContain("sessionStorage");
    expect(migration).not.toMatch(/\bfetch\s*\(/);
    expect(migration).not.toMatch(/\bset(?:Interval|Timeout)\s*\(/);
    expect(migration).not.toMatch(/\b(?:getItem|setItem|removeItem|key)\s*\(/);
    expect(migration).not.toContain("indexedDB");
    expect(migration).not.toContain("@tauri-apps");
    expect(migrationHtml).toContain('<script type="module" src="/src/security/storage-migration.ts"></script>');
    expect(migrationHtml).not.toMatch(/<script(?![^>]*\bsrc=)[^>]*>/);
  });

  it("keeps the thinking-phrase rotation and stopwatch scoped to setInterval alone", () => {
    // A fixed local phrase list cycling, and an elapsed-time readout, on
    // client-side timers while a run block is mounted -- no data in, no data
    // out, cleaned up on unmount.
    const thinkingPhrase = source("features/chat/thinkingPhrase.ts");
    expect(thinkingPhrase).toMatch(/\bsetInterval\s*\(/);
    expect(thinkingPhrase).not.toMatch(/\bfetch\s*\(/);
    expect(thinkingPhrase).not.toContain("localStorage");
    expect(thinkingPhrase).not.toContain("sessionStorage");
    expect(thinkingPhrase).not.toContain("indexedDB");
    expect(thinkingPhrase).not.toContain("@tauri-apps");
  });

  it("keeps the idle sign-out scoped to a local interval and window input events", () => {
    // The inactivity check is a once-a-minute local timer that compares the wall
    // clock with the last input event. It sends nothing, stores nothing and
    // calls no native command itself: signing out is the caller's `onIdle`.
    const idle = source("app/useIdleSignOut.ts");
    expect(idle).toMatch(/\bsetInterval\s*\(/);
    expect(idle).not.toMatch(/\bfetch\s*\(/);
    expect(idle).not.toMatch(/\bsetTimeout\s*\(/);
    expect(idle).not.toContain("localStorage");
    expect(idle).not.toContain("sessionStorage");
    expect(idle).not.toContain("indexedDB");
    expect(idle).not.toContain("@tauri-apps");
    expect(idle).not.toContain("invoke");
  });

  it("contains no raw model-reasoning parser or projection", () => {
    const forbiddenReasoningSurface = [
      "<think>",
      "</think>",
      "thinkingSteps",
      "activeThinking",
      "chain_of_thought",
      "raw_reasoning",
    ];
    const violations = productionSources().flatMap(([path, contents]) => {
      const matches = forbiddenReasoningSurface.filter((term) => contents.includes(term));
      return matches.length > 0 ? [`${path}: ${matches.join(", ")}`] : [];
    });

    expect(violations).toEqual([]);
  });

  it("removes the unsafe starter service modules", () => {
    for (const path of [
      "lib/auth.ts",
      "lib/chat.ts",
      "lib/mcp.ts",
      "lib/upload.ts",
      "lib/planner.ts",
      "lib/permissions.ts",
    ]) {
      expect(sources[`/src/${path}`], path).toBeUndefined();
    }
  });

  it("names no filesystem path to the backend for projects or skills", () => {
    // The backend opens the pickers and reads the drops, and hands the renderer
    // an id. A `path`/`paths` argument would let a compromised renderer choose
    // what the backend acts on.
    const native = source("lib/native.ts");
    expect(native).not.toContain("plugin-dialog");
    expect(native).not.toContain("onDragDropEvent");
    expect(native).not.toMatch(/invoke(?:<[^>]*>)?\(\s*"project_create",\s*\{\s*path\b/);
    expect(native).not.toMatch(/invoke(?:<[^>]*>)?\(\s*"project_find_by_folder",\s*\{\s*path\b/);
    expect(native).not.toMatch(/invoke(?:<[^>]*>)?\(\s*"skill_import",\s*\{\s*paths\b/);
  });

  it("keeps the reusable primitives free of inline styles and style forwarding", () => {
    const primitives = [
      "ui/IconButton.tsx",
      "ui/Menu.tsx",
      "ui/Modal.tsx",
      "ui/SegmentedControl.tsx",
      "ui/Tooltip.tsx",
      "ui/StatusDot.tsx",
    ];

    for (const path of primitives) {
      expect(source(path), path).not.toMatch(/\bstyle\s*=/);
    }
    for (const path of ["ui/IconButton.tsx", "ui/StatusDot.tsx"]) {
      expect(source(path), path).not.toMatch(/\bstyle\s*(?:[,}:])/);
    }
  });
});
