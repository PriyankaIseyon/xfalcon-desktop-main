/* global window, document, MutationObserver, requestAnimationFrame -- the page.evaluate and init-script code runs in Chromium */
// Startup benchmark: cold page load of a built dist against the mocked native boundary.
// usage: node scripts/perf/startup.mjs [--dist dist] [--runs 9] [--throttle 4] [--label x] [--port 0] [--json out.json]
import { writeFileSync } from "node:fs";
import { chromium } from "@playwright/test";
import { arg, intArg, numArg } from "./args.mjs";
import { serve, resolveDist } from "./serve.mjs";
import { installPerfMock } from "./mock.mjs";
import { median } from "./stats.mjs";

// All flags are read and validated here, before the server or the browser starts.
const dist = resolveDist(arg("dist", "dist"));
const runs = intArg("runs", 9, 1);
const throttle = numArg("throttle", 4, 1);
const label = arg("label", "run");
const port = intArg("port", 0, 0, 65535);
const json = arg("json");

const p90 = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor(s.length * 0.9))];
};

let server;
let browser;
try {
  const started = await serve(dist, port);
  server = started.server;
  browser = await chromium.launch({ args: ["--disable-features=Translate", "--no-sandbox"] });
  const samples = [];

  for (let i = 0; i < runs + 1; i++) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await context.newPage();
    await installPerfMock(page, { conversations: 300 });
    await page.addInitScript(() => {
      window.__marks = {};
      const want = {
        shell: 'nav[aria-label="Workspace"]',
        composer: 'form[aria-label="Message composer"] textarea',
        conversation: '[role="option"]',
      };
      const check = () => {
        for (const [k, sel] of Object.entries(want)) {
          if (!(k in window.__marks) && document.querySelector(sel)) window.__marks[k] = performance.now();
        }
      };
      new MutationObserver(check).observe(document, { childList: true, subtree: true });
      window.__longTasks = [];
      try {
        new PerformanceObserver((l) => {
          for (const e of l.getEntries()) window.__longTasks.push(e.duration);
        }).observe({ type: "longtask", buffered: true });
      } catch {
        /* longtask observer is optional */
      }
    });
    const cdp = await context.newCDPSession(page);
    await cdp.send("Performance.enable");
    if (throttle > 1) await cdp.send("Emulation.setCPUThrottlingRate", { rate: throttle });
    await page.goto(`http://127.0.0.1:${started.port}/`, { waitUntil: "load" });
    await page.waitForFunction(() => "composer" in window.__marks && "conversation" in window.__marks, null, {
      timeout: 30_000,
    });
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
    const metrics = Object.fromEntries(
      (await cdp.send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]),
    );
    const data = await page.evaluate(() => {
      const nav = performance.getEntriesByType("navigation")[0];
      const paint = Object.fromEntries(performance.getEntriesByType("paint").map((p) => [p.name, p.startTime]));
      const res = performance.getEntriesByType("resource");
      const sum = (f) => res.filter(f).reduce((n, r) => n + (r.encodedBodySize || 0), 0);
      return {
        domContentLoaded: nav.domContentLoadedEventEnd,
        load: nav.loadEventEnd,
        fcp: paint["first-contentful-paint"],
        shell: window.__marks.shell,
        composer: window.__marks.composer,
        conversation: window.__marks.conversation,
        longTaskMs: window.__longTasks.reduce((a, b) => a + b, 0),
        longTasks: window.__longTasks.length,
        jsBytes: sum((r) => r.name.endsWith(".js")),
        cssBytes: sum((r) => r.name.endsWith(".css")),
        fontBytes: sum((r) => /\.(ttf|woff2?)$/.test(r.name)),
        requests: res.length,
        ipcCalls: window.__PERF_MOCK__.invocations.length,
        unknown: [...window.__PERF_MOCK__.unknown],
      };
    });
    data.scriptMs = metrics.ScriptDuration * 1000;
    data.layoutMs = metrics.LayoutDuration * 1000;
    data.styleMs = metrics.RecalcStyleDuration * 1000;
    data.taskMs = metrics.TaskDuration * 1000;
    data.heapMB = metrics.JSHeapUsedSize / 1e6;
    data.nodes = metrics.Nodes;
    if (i > 0) samples.push(data); // run 0 is warm-up for the browser itself, discarded
    await context.close();
  }

  const keys = [
    "fcp",
    "domContentLoaded",
    "load",
    "shell",
    "composer",
    "conversation",
    "scriptMs",
    "layoutMs",
    "styleMs",
    "taskMs",
    "longTaskMs",
    "longTasks",
    "heapMB",
    "nodes",
  ];
  const out = { label, runs, throttle, medians: {}, p90: {}, static: {} };
  for (const k of keys) {
    out.medians[k] = +median(samples.map((s) => s[k])).toFixed(1);
    out.p90[k] = +p90(samples.map((s) => s[k])).toFixed(1);
  }
  for (const k of ["jsBytes", "cssBytes", "fontBytes", "requests", "ipcCalls"]) out.static[k] = samples[0][k];
  out.unknownCommands = [...new Set(samples.flatMap((s) => s.unknown))].sort();
  console.log(JSON.stringify(out, null, 2));
  if (json) writeFileSync(json, JSON.stringify({ ...out, samples }, null, 2));
} catch (e) {
  console.error(`startup failed: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser?.close();
  server?.close();
}
