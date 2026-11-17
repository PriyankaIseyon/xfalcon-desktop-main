/* global document, MutationObserver, requestAnimationFrame, window -- the page.evaluate and init-script code runs in Chromium */
// Screen-open benchmark: click each destination, measure click -> first paint and click -> DOM settled.
// usage: node scripts/perf/screens.mjs [--dist dist] [--only Projects,Datasets,...] [--runs 7] [--throttle 4] [--label x]
//        [--ipcDelay 0] [--ipcJitter 0] [--conversations 300] [--dirEntries 4000] [--datasets 200] [--reports 200]
//        [--port 0] [--json out.json]
// --only takes destination names (Projects, Datasets, Reports, Chats, Settings). A repeated name is measured as a revisit.
import { writeFileSync } from "node:fs";
import { chromium } from "@playwright/test";
import { arg, intArg, numArg, usageError } from "./args.mjs";
import { serve, resolveDist } from "./serve.mjs";
import { installPerfMock } from "./mock.mjs";
import { median } from "./stats.mjs";

// All flags are read and validated here, before the server or the browser starts.
const dist = resolveDist(arg("dist", "dist"));
const runs = intArg("runs", 7, 1);
const throttle = numArg("throttle", 4, 1);
const label = arg("label", "run");
const port = intArg("port", 0, 0, 65535);
const json = arg("json");

const DESTINATIONS = ["Projects", "Datasets", "Reports", "Chats", "Settings"];
const onlyArg = arg("only");
const SEQUENCE =
  onlyArg === undefined
    ? ["Projects", "Datasets", "Reports", "Chats", "Projects", "Datasets", "Reports", "Settings"]
    : onlyArg.split(",");
const unknownDest = SEQUENCE.filter((n) => !DESTINATIONS.includes(n));
if (unknownDest.length) usageError(`unknown screen ${unknownDest.join(",")}; known: ${DESTINATIONS.join(",")}`);
const mockCfg = {
  ipcDelay: numArg("ipcDelay", 0, 0),
  ipcJitter: numArg("ipcJitter", 0, 0),
  conversations: intArg("conversations", 300, 0),
  dirEntries: intArg("dirEntries", 4000, 0),
  datasets: intArg("datasets", 200, 0),
  reports: intArg("reports", 200, 0),
};

let server;
let browser;
try {
  const started = await serve(dist, port);
  server = started.server;
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  const bucket = {}; // key -> {paint:[], content:[], settled:[], busy:[], nodes:[]}
  const unknownSeen = new Set();

  // Pass 0 is a discarded warm-up over the whole sequence; passes 1..runs are measured.
  for (let i = 0; i < runs + 1; i++) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await context.newPage();
    await installPerfMock(page, mockCfg);
    const cdp = await context.newCDPSession(page);
    await cdp.send("Performance.enable");
    await page.goto(`http://127.0.0.1:${started.port}/`, { waitUntil: "load" });
    await page.waitForSelector('nav[aria-label="Workspace"]');
    await page.waitForSelector('[role="option"]');
    await page.waitForTimeout(500);
    if (throttle > 1) await cdp.send("Emulation.setCPUThrottlingRate", { rate: throttle });
    const seen = new Set();
    for (const name of SEQUENCE) {
      const before = Object.fromEntries(
        (await cdp.send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]),
      );
      const r = await page.evaluate(async (dest) => {
        const btn = [...document.querySelectorAll("button.nav-destination, button.nav-item--destination")].find((b) =>
          (b.getAttribute("aria-label") || b.textContent || "").trim().startsWith(dest),
        );
        if (!btn) return { error: `no button ${dest}` };
        const t0 = performance.now();
        // Starts at the click, so a click that changes no DOM reports 0 settled time.
        let last = t0;
        const mo = new MutationObserver(() => {
          last = performance.now();
        });
        mo.observe(document.body, { childList: true, subtree: true, attributes: true, characterData: true });
        btn.click();
        await new Promise((res) => requestAnimationFrame(() => requestAnimationFrame(res)));
        const paint = performance.now() - t0;
        let contentAt = null;
        const markers = {
          Projects: "file-1.ts",
          Datasets: "sales_2026_001",
          Reports: "Revenue dashboard 1",
          Chats: "Quarterly revenue analysis number 5",
          Settings: "Settings",
        };
        const marker = markers[dest];
        for (let k = 0; k < 600; k++) {
          if (document.body.textContent.includes(marker)) {
            contentAt = performance.now() - t0;
            break;
          }
          await new Promise((res) => setTimeout(res, 4));
        }
        await new Promise((res) => setTimeout(res, 400));
        mo.disconnect();
        return {
          paint,
          content: contentAt, // null when the marker never showed in this run
          settled: Math.max(0, last - t0),
          nodes: document.getElementsByTagName("*").length,
        };
      }, name);
      const after = Object.fromEntries(
        (await cdp.send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]),
      );
      if (r.error) throw new Error(r.error);
      r.busy = (after.TaskDuration - before.TaskDuration) * 1000; // main-thread task time; the 400ms idle wait adds ~0
      const key = `${name}:${seen.has(name) ? "revisit" : "first"}`;
      seen.add(name);
      if (i > 0) {
        const b = (bucket[key] ??= { paint: [], content: [], settled: [], busy: [], nodes: [] });
        b.content.push(r.content);
        b.paint.push(r.paint);
        b.settled.push(r.settled);
        b.busy.push(r.busy);
        b.nodes.push(r.nodes);
      }
    }
    for (const c of await page.evaluate(() => [...window.__PERF_MOCK__.unknown])) unknownSeen.add(c);
    await context.close();
  }

  const out = { label, runs, throttle, screens: {} };
  for (const [k, b] of Object.entries(bucket)) {
    const found = b.content.filter((v) => v !== null);
    if (found.length === 0) throw new Error(`content marker for ${k} never appeared in any measured run`);
    out.screens[k] = {
      paintMs: +median(b.paint).toFixed(1),
      contentMs: +median(found).toFixed(1),
      settledMs: +median(b.settled).toFixed(1),
      busyMs: +median(b.busy).toFixed(1),
      nodes: median(b.nodes),
    };
  }
  out.unknownCommands = [...unknownSeen].sort();
  console.log(JSON.stringify(out, null, 2));
  if (json) writeFileSync(json, JSON.stringify({ ...out, bucket }, null, 2));
} catch (e) {
  console.error(`screens failed: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser?.close();
  server?.close();
}
