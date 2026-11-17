// Minimal static file server for a built dist directory, with the content types and no caching surprises.
import http from "node:http";
import { existsSync } from "node:fs";
import { readFile, stat } from "node:fs/promises";
import { extname, join, normalize, resolve, sep } from "node:path";

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".woff2": "font/woff2",
  ".ttf": "font/ttf",
  ".json": "application/json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".txt": "text/plain",
};

// Resolves a --dist path against the working directory. Exits early when it is not a Vite build output.
export function resolveDist(dist) {
  const root = resolve(process.cwd(), dist);
  if (!existsSync(join(root, "index.html"))) {
    console.error(
      `dist directory ${root} does not exist or has no index.html. Run "npx vite build --outDir <dir>" first.`,
    );
    process.exit(1);
  }
  return root;
}

// Serves root on 127.0.0.1. Port 0 picks a free port. Resolves { server, port } with the bound port.
// Range requests are not supported: every file is answered 200 with the whole body. Playwright page loads do not send Range.
export function serve(root, port = 0) {
  const server = http.createServer(async (req, res) => {
    try {
      const url = new URL(req.url, "http://x");
      let p = normalize(decodeURIComponent(url.pathname));
      if (p.endsWith("/")) p += "index.html";
      let file = join(root, p);
      if (file !== root && !file.startsWith(root + sep)) {
        res.writeHead(403).end();
        return;
      }
      const info = await stat(file).catch(() => null);
      if (info?.isDirectory()) {
        file = join(file, "index.html");
      } else if (!info) {
        // A missing file with an extension is a missing asset. Extension-less paths are client routes.
        if (extname(p)) {
          res.writeHead(404).end("not found");
          return;
        }
        file = join(root, "index.html");
      }
      const body = await readFile(file);
      res.writeHead(200, {
        "content-type": TYPES[extname(file)] ?? "application/octet-stream",
        "cache-control": "no-store",
      });
      res.end(body);
    } catch (e) {
      res.writeHead(404).end(String(e));
    }
  });
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve({ server, port: server.address().port }));
  });
}
