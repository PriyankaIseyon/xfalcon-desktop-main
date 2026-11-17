/* global window, document, requestAnimationFrame, getComputedStyle -- the page.evaluate and init-script code runs in Chromium */
// CPU-profile one destination click (or a streamHtml / scrollHeavy run), resolved through the sourcemap when the build has one.
// --dest takes a nav destination (Projects, Datasets, Reports, Chats, Settings), streamHtml or scrollHeavy.
// --cfg is a JSON object of mock options (see mock.mjs), e.g. '{"datasets":200}'. --json writes the raw cpuprofile.
// usage: node scripts/perf/profile.mjs [--dist dist] [--dest Datasets] [--throttle 1] [--wait 3000] [--cfg '{}'] [--top 22] [--port 0] [--json out.cpuprofile]
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { chromium } from "@playwright/test";
import { arg, intArg, numArg, usageError } from "./args.mjs";
import { serve, resolveDist } from "./serve.mjs";
import { installPerfMock, MOCK_DEFAULTS } from "./mock.mjs";

// All flags are read and validated here, before the server or the browser starts.
const dist = resolveDist(arg("dist", "dist"));
const DESTINATIONS = ["Projects", "Datasets", "Reports", "Chats", "Settings", "streamHtml", "scrollHeavy"];
const dest = arg("dest", "Datasets");
if (!DESTINATIONS.includes(dest)) usageError(`unknown --dest ${dest}; known: ${DESTINATIONS.join(", ")}`);
const cfgText = arg("cfg", "{}");
let cfg;
try {
  cfg = JSON.parse(cfgText);
} catch (e) {
  usageError(`--cfg must be a JSON object: ${e.message}`);
}
if (typeof cfg !== "object" || cfg === null || Array.isArray(cfg)) usageError("--cfg must be a JSON object");
const unknownKeys = Object.keys(cfg).filter((k) => !(k in MOCK_DEFAULTS));
if (unknownKeys.length)
  usageError(`unknown --cfg key ${unknownKeys.join(",")}; known: ${Object.keys(MOCK_DEFAULTS).join(",")}`);
const throttle = numArg("throttle", 1, 1);
const wait = numArg("wait", 3000, 0);
const top = intArg("top", 22, 1);
const port = intArg("port", 0, 0, 65535);
const json = arg("json");

let server;
let browser;
try {
  const started = await serve(dist, port);
  server = started.server;
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  await installPerfMock(page, cfg);
  const cdp = await context.newCDPSession(page);
  await page.goto(`http://127.0.0.1:${started.port}/`, { waitUntil: "load" });
  await page.waitForSelector('[role="option"]');
  await page.waitForTimeout(500);
  if (throttle > 1) await cdp.send("Emulation.setCPUThrottlingRate", { rate: throttle });
  await cdp.send("Profiler.enable");
  await cdp.send("Profiler.setSamplingInterval", { interval: 100 });
  if (dest === "streamHtml") {
    await page.evaluate(() => document.querySelector('[role="option"]').click());
    await page.waitForSelector(".chat-column-messages [data-index]", { timeout: 60000 });
    await page.waitForTimeout(800);
    await cdp.send("Profiler.start");
    await page.evaluate(async () => {
      const kb = 60;
      const card = (i) =>
        `<div class="card"><h3>Region ${i}</h3><table><tr><th>Metric</th><th>Value</th></tr><tr><td>Revenue</td><td>${i * 1000}</td></tr><tr><td>Margin</td><td>${i}%</td></tr></table><p style="color:#333">Commentary for region ${i}: revenue grew while margin compressed.</p></div>\n`;
      let body = "";
      for (let i = 0; body.length < kb * 1024; i++) body += card(i);
      const text =
        'Here is the dashboard:\n\n```html\n<!DOCTYPE html>\n<html><head><meta charset="utf-8"><style>.card{border:1px solid #ccc;padding:8px;margin:4px}</style></head><body>\n' +
        body +
        "</body></html>\n```\n\nLet me know what to change.";
      const emit = window.__PERF_MOCK__.emit;
      const CH = "xfalcon://chat-inference-event";
      const gaps = [];
      let longMs = 0;
      let longCount = 0;
      const po = new PerformanceObserver((l) => {
        for (const e of l.getEntries()) {
          longMs += e.duration;
          longCount += 1;
        }
      });
      try {
        po.observe({ type: "longtask" });
      } catch {
        /* longtask observer is optional */
      }
      let prev = performance.now();
      let seq = 0;
      const t0 = performance.now();
      for (let off = 0; off < text.length; off += 512) {
        await new Promise((res) =>
          requestAnimationFrame((t) => {
            gaps.push(t - prev);
            prev = t;
            res();
          }),
        );
        emit(CH, {
          type: "text_delta",
          version: 1,
          run_id: "run-1",
          sequence: ++seq,
          text: text.slice(off, off + 512),
        });
      }
      await new Promise((res) => setTimeout(res, 300));
      po.disconnect();
      gaps.sort((a, b) => a - b);
      return {
        totalMs: performance.now() - t0,
        frames: gaps.length,
        p50: gaps[Math.floor(gaps.length * 0.5)],
        p95: gaps[Math.floor(gaps.length * 0.95)],
        max: gaps[gaps.length - 1],
        longMs,
        longCount,
      };
    });
  } else if (dest === "scrollHeavy") {
    await page.evaluate(() => document.querySelector('[role="option"]').click());
    await page.waitForSelector(".chat-column-messages [data-index]", { timeout: 60000 });
    await page.waitForTimeout(1500);
    await cdp.send("Profiler.start");
    await page.evaluate(async () => {
      let e = document.querySelector(".chat-column-messages [data-index]");
      while (e && !(e.scrollHeight > e.clientHeight + 50 && /(auto|scroll)/.test(getComputedStyle(e).overflowY)))
        e = e.parentElement;
      for (let i = 0; i < 240; i++)
        await new Promise((r) =>
          requestAnimationFrame(() => {
            e.scrollTop += i < 120 ? -500 : 500;
            r();
          }),
        );
    });
  } else {
    await cdp.send("Profiler.start");
    await page.evaluate((d) => {
      const btn = [...document.querySelectorAll("button.nav-destination")].find((b) =>
        b.textContent.trim().startsWith(d),
      );
      if (!btn)
        throw new Error(
          `no destination ${d}; use Projects, Datasets, Reports, Chats, Settings, streamHtml or scrollHeavy`,
        );
      btn.click();
    }, dest);
    await page.waitForTimeout(wait);
  }
  const { profile } = await cdp.send("Profiler.stop");
  const unknownCommands = (await page.evaluate(() => [...window.__PERF_MOCK__.unknown])).sort();
  await browser.close();
  browser = undefined;
  server.close();
  server = undefined;

  // sourcemap resolver: needs a build with sourcemaps (for example vite build --sourcemap); otherwise frames stay bundle lines
  const assets = join(dist, "assets");
  const mapFile = existsSync(assets) ? readdirSync(assets).find((f) => /^main-.*\.js\.map$/.test(f)) : undefined;
  const map = mapFile ? JSON.parse(readFileSync(join(assets, mapFile), "utf8")) : null;
  const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const dec = (s) => {
    const out = [];
    let shift = 0,
      val = 0;
    for (const ch of s) {
      const d = B64.indexOf(ch);
      val += (d & 31) << shift;
      if (d & 32) {
        shift += 5;
      } else {
        out.push(val & 1 ? -(val >> 1) : val >> 1);
        shift = 0;
        val = 0;
      }
    }
    return out;
  };
  let table = null;
  if (map) {
    let src = 0,
      sl = 0,
      sc = 0;
    table = map.mappings.split(";").map((line) => {
      let col = 0;
      const out = [];
      if (line)
        for (const seg of line.split(",")) {
          const v = dec(seg);
          col += v[0];
          if (v.length >= 4) {
            src += v[1];
            sl += v[2];
            sc += v[3];
            out.push([col, src, sl, sc]);
          }
        }
      return out;
    });
  }
  const resolveFrame = (l, c) => {
    if (!table) return null;
    let best = null;
    for (const s of table[l] ?? []) if (s[0] <= c) best = s;
    return best ? `${map.sources[best[1]].replace(/^(\.\.\/)+/, "")}:${best[2] + 1}` : null;
  };

  const byId = new Map(profile.nodes.map((n) => [n.id, n]));
  const self = new Map();
  profile.samples.forEach((id, i) => {
    const f = byId.get(id).callFrame;
    const where = f.url.includes("/assets/main") ? resolveFrame(f.lineNumber, f.columnNumber) : null;
    const key = `${f.functionName || "(anon)"} ${where ?? f.url.split("/").pop() + ":" + (f.lineNumber + 1)}`;
    // timeDeltas[i] is the gap before sample i, so sample i ran until sample i + 1 started. The last sample is charged 0.
    const ran = i + 1 < profile.timeDeltas.length ? profile.timeDeltas[i + 1] : 0;
    self.set(key, (self.get(key) || 0) + ran / 1000);
  });
  const busy = [...self].filter(([k]) => !k.startsWith("(idle)")).sort((a, b) => b[1] - a[1]);
  const total = busy.reduce((a, [, v]) => a + v, 0);
  const shown = busy.slice(0, top).map(([fn, ms]) => ({ fn, ms: +ms.toFixed(1) }));
  console.log(
    JSON.stringify(
      {
        dest,
        throttle,
        busyMs: +total.toFixed(0),
        sourcemap: Boolean(map),
        top: shown,
        unknownCommands,
      },
      null,
      2,
    ),
  );
  if (json) writeFileSync(json, JSON.stringify(profile));
} catch (e) {
  console.error(`profile failed: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser?.close();
  server?.close();
}
