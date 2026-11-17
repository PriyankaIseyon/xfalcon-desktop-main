import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { basename, dirname, resolve, sep } from "node:path";

/**
 * A checkout on a Windows drive (`/mnt/<letter>/…`) served by a dev server
 * running inside WSL gets no inotify events for edits made on either side, so
 * the watcher silently goes stale: the server keeps serving the module it read
 * at boot while the file on disk moves on. Nothing reports this — the page just
 * stops reflecting edits, which reads as "the app is broken" rather than "the
 * watcher is deaf". Polling is the documented remedy; it costs CPU, so it is
 * enabled only in the arrangement that needs it, and never on native Linux,
 * macOS, or Windows.
 */
// `@types/node` is not a dependency here, and pulling it in to read two values
// would be a heavier change than declaring exactly what this file uses.
declare const process: { env: Record<string, string | undefined>; cwd(): string };

const isWslOnWindowsDrive =
  Boolean(process.env.WSL_DISTRO_NAME) && process.cwd().startsWith("/mnt/");

const require = createRequire(import.meta.url);
const pdfjsRoot = dirname(require.resolve("pdfjs-dist/package.json"));
const pdfAssetGroups = ["cmaps", "standard_fonts"] as const;
const pdfWorkerPath = resolve(pdfjsRoot, "legacy/build/pdf.worker.mjs");

function officePdfStaticAssets(): Plugin {
  const files = pdfAssetGroups.flatMap((group) =>
    readdirSync(resolve(pdfjsRoot, group))
      .filter((file) => statSync(resolve(pdfjsRoot, group, file)).isFile())
      .map((file) => ({ group, file })),
  );

  return {
    name: "office-pdf-static-assets",
    configureServer(server) {
      server.middlewares.use("/pdfjs-assets", (request, response) => {
        let pathname: string;
        try {
          pathname = new URL(request.url ?? "/", "http://vite.local").pathname;
        } catch {
          response.statusCode = 400;
          response.end("Invalid PDF asset path.");
          return;
        }
        const parts = pathname.split("/").filter(Boolean);
        if (parts[0] === "pdfjs-assets") parts.shift();
        if (parts.length === 1 && parts[0] === "pdf.worker.mjs") {
          response.statusCode = 200;
          response.setHeader("Content-Type", "text/javascript; charset=utf-8");
          response.setHeader("X-Content-Type-Options", "nosniff");
          response.setHeader("Cache-Control", "no-store");
          response.end(readFileSync(pdfWorkerPath));
          return;
        }
        if (parts.length !== 2 || !pdfAssetGroups.includes(parts[0] as (typeof pdfAssetGroups)[number])) {
          response.statusCode = 404;
          response.end("PDF asset not found.");
          return;
        }
        const filePath = resolve(pdfjsRoot, parts[0], parts[1]);
        if (!filePath.startsWith(`${resolve(pdfjsRoot)}${sep}`) || basename(filePath) !== parts[1]) {
          response.statusCode = 403;
          response.end("Invalid PDF asset path.");
          return;
        }
        try {
          const content = readFileSync(filePath);
          const extension = parts[1].slice(parts[1].lastIndexOf(".")).toLowerCase();
          const contentType =
            extension === ".ttf"
              ? "font/ttf"
              : extension === ".bcmap" || extension === ".pfb"
                ? "application/octet-stream"
                : "text/plain; charset=utf-8";
          response.statusCode = 200;
          response.setHeader("Content-Type", contentType);
          response.setHeader("X-Content-Type-Options", "nosniff");
          response.setHeader("Cache-Control", "no-store");
          response.end(content);
        } catch {
          response.statusCode = 404;
          response.end("PDF asset not found.");
        }
      });
    },
    generateBundle() {
      this.emitFile({
        type: "asset",
        fileName: "pdfjs-assets/pdf.worker.mjs",
        source: readFileSync(pdfWorkerPath),
      });
      for (const { group, file } of files) {
        this.emitFile({
          type: "asset",
          fileName: `pdfjs-assets/${group}/${file}`,
          source: readFileSync(resolve(pdfjsRoot, group, file)),
        });
      }
    },
  };
}

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss(), officePdfStaticAssets()],
  define: {
    "import.meta.env.VITE_APP_REVISION": JSON.stringify(
      execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
    ),
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    // The loopback literal, not the name: `localhost` resolves to `::1` first on
    // Windows, and a WSL-hosted dev server listening only on IPv4 turns that
    // into a hang rather than a refusal — the webview never loads and the app
    // exits on its startup deadline with nothing logged. `127.0.0.1` is the
    // address that is actually listening, and `is_exact_app_origin` already
    // accepts it as a development origin.
    host: "127.0.0.1",
    hmr: {
      protocol: "ws",
      host: "127.0.0.1",
      port: 1421,
    },
    // Polling has to be scoped or it is worse than the problem it solves: this
    // tree holds a multi-gigabyte Rust `target/` directory, and stat-ing all of
    // it on an interval over drvfs pegs the dev server's event loop — requests
    // then sit unread in the socket's receive queue and the app's webview never
    // finishes loading. Only the directories a source edit can land in are
    // watched.
    watch: isWslOnWindowsDrive
      ? {
          usePolling: true,
          interval: 1000,
          binaryInterval: 2000,
          ignored: [
            "**/node_modules/**",
            "**/.git/**",
            "**/target/**",
            "**/dist/**",
            "**/.worktrees/**",
            "**/test-results/**",
            "**/playwright-report/**",
          ],
        }
      : undefined,
  },
  // 3. to make use of `TAURI_ENV_*` env variables
  // https://tauri.app/v1/guides/features/environment-variables/
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    rollupOptions: {
      input: {
        main: "index.html",
        storageMigration: "storage-migration.html",
      },
    },
  },
}));
