/* global window, document, HTMLInputElement, requestAnimationFrame, -- the page.evaluate and init-script code runs in Chromium */
// Interaction scenarios: conversation open, transcript scroll, sidebar scroll, projects filter keystroke.
// Default --only runs 5 scenarios: open200,open10k,scroll10k,sidebar1500,filter20k. Also accepted: csvOpenFilter,
// palette2k, streamHtml, newChatSend, scrollHeavy. run.mjs passes all ten.
// Every selected scenario runs once as a discarded warm-up, then --runs times. A scenario whose scroller or
// marker is missing throws within its deadline, so the run exits 1 instead of reporting an empty result.
// usage: node scripts/perf/scenarios.mjs [--dist dist] [--only open200,open10k,...] [--runs 5] [--throttle 4] [--label x]
//        [--port 0] [--json out.json]
import { writeFileSync } from "node:fs";
import { chromium } from "@playwright/test";
import { arg, intArg, numArg, usageError } from "./args.mjs";
import { serve, resolveDist } from "./serve.mjs";
import { installPerfMock } from "./mock.mjs";
import { median } from "./stats.mjs";

// All flags are read and validated here, before the server or the browser starts.
const dist = resolveDist(arg("dist", "dist"));
const runs = intArg("runs", 5, 1);
const throttle = numArg("throttle", 4, 1);
const label = arg("label", "run");
const onlyArg = arg("only", "open200,open10k,scroll10k,sidebar1500,filter20k");
const only = [...new Set(onlyArg.split(","))];
const port = intArg("port", 0, 0, 65535);
const json = arg("json");

let server;
let browser;
let base;
const unknownSeen = new Set();
async function closeRun(page, context) {
  for (const c of await page.evaluate(() => [...window.__PERF_MOCK__.unknown])) unknownSeen.add(c);
  await context.close();
}

async function boot(cfg) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  await installPerfMock(page, cfg);
  const cdp = await context.newCDPSession(page);
  await cdp.send("Performance.enable");
  await page.goto(base, { waitUntil: "load" });
  await page.waitForSelector('[role="option"]');
  await page.waitForTimeout(400);
  if (throttle > 1) await cdp.send("Emulation.setCPUThrottlingRate", { rate: throttle });
  return { context, page, cdp };
}
const metrics = async (cdp) =>
  Object.fromEntries((await cdp.send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]));

// In-page wait, shared by the scenarios that wait for rendered content. Resolves once check() is true. Rejects with
// the name of what it waited for when the deadline passes, so a missing row fails the run.
const WAIT_FN = `((check, what, timeoutMs = 60000) => new Promise((resolve, reject) => {
  const end = performance.now() + timeoutMs;
  const tick = () => {
    if (check()) resolve();
    else if (performance.now() > end) reject(new Error("timed out waiting for " + what));
    else setTimeout(tick, 5);
  };
  tick();
}))`;

// scroll helper: drive scrollTop in rAF steps, record frame gaps and long tasks. Throws when there is no scroller.
const SCROLL_FN = `(async ({ selector, steps, delta }) => {
  const el = typeof selector === "string" ? document.querySelector(selector) : selector;
  if (!el) throw new Error("no scrollable element found for the scroll scenario");
  const gaps = []; let longMs = 0; let longCount = 0;
  const po = new PerformanceObserver((l) => { for (const e of l.getEntries()) { longMs += e.duration; longCount += 1; } });
  try { po.observe({ type: "longtask" }); } catch { /* longtask observer is optional */ }
  let prev = performance.now();
  for (let i = 0; i < steps; i++) {
    await new Promise((r) => requestAnimationFrame((t) => { gaps.push(t - prev); prev = t; el.scrollTop += delta; r(); }));
  }
  await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  po.disconnect();
  gaps.sort((a, b) => a - b);
  return { p50: gaps[Math.floor(gaps.length * 0.5)], p95: gaps[Math.floor(gaps.length * 0.95)], max: gaps[gaps.length - 1], longMs, longCount, frames: gaps.length };
})`;

const scenarios = {
  async csvOpenFilter() {
    const { context, page } = await boot({ dirEntries: 3000 });
    await page.evaluate(() =>
      [...document.querySelectorAll("button.nav-destination")]
        .find((b) => b.textContent.trim().startsWith("Projects"))
        .click(),
    );
    await page.waitForSelector('[aria-label="Filter visible project files"]', { timeout: 60000 });
    await page.waitForTimeout(2500);
    const setFilter = (v) =>
      page.evaluate((value) => {
        const input = document.querySelector('[aria-label="Filter visible project files"]');
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      }, v);
    await setFilter("part-00001.csv");
    await page.waitForTimeout(1500);
    const open = await page.evaluate(`(async () => {
      const t0 = performance.now();
      const row = [...document.querySelectorAll("[role=treeitem]")].find((e) =>
        e.textContent.includes("part-00001.csv"),
      );
      if (!row) throw new Error("no CSV row part-00001.csv in the filtered tree");
      row.click();
      await ${WAIT_FN}(() => document.body.textContent.includes("North 1"), "the CSV preview rows");
      return { openMs: performance.now() - t0 };
    })()`);
    await page.waitForTimeout(1000);
    await setFilter("");
    await page.waitForTimeout(1500);
    const times = [];
    for (const ch of "part-0") {
      const t = await page.evaluate(async (c) => {
        const input = document.querySelector('[aria-label="Filter visible project files"]');
        const t0 = performance.now();
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
        setter.call(input, input.value + c);
        input.dispatchEvent(new Event("input", { bubbles: true }));
        await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
        return performance.now() - t0;
      }, ch);
      times.push(t);
    }
    await closeRun(page, context);
    return { openMs: open.openMs, keystrokeMs: median(times) };
  },
  async palette2k() {
    const { context, page } = await boot({ messages: 2000 });
    await page.evaluate(() => document.querySelector('[role="option"]').click());
    await page.waitForSelector(".chat-column-messages [data-index]", { timeout: 60000 });
    await page.waitForTimeout(800);
    await page.keyboard.press("Control+k");
    await page.waitForSelector(
      '[role="dialog"] input, [role="combobox"], input[type=search], input[aria-label*=earch]',
      { timeout: 10000 },
    );
    const times = [];
    for (const ch of "revenue") {
      const r = await page.evaluate(async (c) => {
        const input = document.activeElement;
        const t0 = performance.now();
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
        setter.call(input, input.value + c);
        input.dispatchEvent(new Event("input", { bubbles: true }));
        await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
        return performance.now() - t0;
      }, ch);
      times.push(r);
    }
    await closeRun(page, context);
    return { keystrokeMs: median(times), firstMs: times[0] };
  },
  async open200() {
    return openConversation(200);
  },
  async open10k() {
    return openConversation(10000);
  },
  async scroll10k() {
    const { context, page } = await boot({ messages: 10000 });
    await page.evaluate(() => document.querySelector('[role="option"]').click());
    await page.waitForSelector(".chat-column-messages [data-index]", { timeout: 60000 });
    await page.waitForTimeout(800);
    const r = await page.evaluate(
      `${SCROLL_FN}({ selector: (() => { let e = document.querySelector('.chat-column-messages [data-index]'); while (e && !(e.scrollHeight > e.clientHeight + 50 && /(auto|scroll)/.test(getComputedStyle(e).overflowY))) e = e.parentElement; return e; })(), steps: 150, delta: 400 })`,
    );
    await closeRun(page, context);
    return r;
  },
  async streamHtml() {
    const { context, page } = await boot({ messages: 20, activeRunId: "run-1" });
    await page.evaluate(() => document.querySelector('[role="option"]').click());
    await page.waitForSelector(".chat-column-messages [data-index]", { timeout: 60000 });
    await page.waitForTimeout(800);
    const r = await page.evaluate(async () => {
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
    await closeRun(page, context);
    return r;
  },
  async newChatSend() {
    const { context, page } = await boot({ ipcDelay: 120, conversations: 50 });
    await page.evaluate(() => {
      window.__PERF_MOCK__.invocations.length = 0;
    });
    const box = page.locator('form[aria-label="Message composer"] textarea');
    await box.click();
    await box.fill("What changed in revenue last week?");
    const t0 = await page.evaluate(() => performance.now());
    await page.keyboard.press("Enter");
    await page.waitForFunction(
      () => window.__PERF_MOCK__.invocations.some((i) => i.cmd === "chat_message_send"),
      null,
      { timeout: 20000 },
    );
    const inv = await page.evaluate(() =>
      window.__PERF_MOCK__.invocations.filter((i) => !i.cmd.startsWith("plugin:")).map((i) => [i.cmd, Math.round(i.t)]),
    );
    const sendT = inv.find(([c]) => c === "chat_message_send")[1];
    const order = inv.map(([c]) => c);
    await closeRun(page, context);
    // sendIssuedMs is stamped in the mock when chat_message_send reaches the native side, after its simulated IPC
    // delay (120 ms here). It therefore includes that delay for every native call the app awaited before the send.
    // It is not the time from the key press to the first app work.
    return { sendIssuedMs: sendT - t0, orderedBeforeSend: order.slice(0, order.indexOf("chat_message_send")).length };
  },
  async scrollHeavy() {
    const { context, page, cdp } = await boot({ messages: 400, heavy: true });
    await page.evaluate(() => document.querySelector('[role="option"]').click());
    await page.waitForSelector(".chat-column-messages [data-index]", { timeout: 60000 });
    await page.waitForTimeout(1500);
    const find = `(() => { let e = document.querySelector('.chat-column-messages [data-index]'); while (e && !(e.scrollHeight > e.clientHeight + 50 && /(auto|scroll)/.test(getComputedStyle(e).overflowY))) e = e.parentElement; return e; })()`;
    // scroll up through history (older content), then back down over the same rows (remount)
    const m0 = await metrics(cdp);
    const up = await page.evaluate(`${SCROLL_FN}({ selector: ${find}, steps: 120, delta: -500 })`);
    const down = await page.evaluate(`${SCROLL_FN}({ selector: ${find}, steps: 120, delta: 500 })`);
    const m1 = await metrics(cdp);
    await closeRun(page, context);
    return {
      scriptMs: (m1.ScriptDuration - m0.ScriptDuration) * 1000,
      layoutMs: (m1.LayoutDuration - m0.LayoutDuration) * 1000,
      styleMs: (m1.RecalcStyleDuration - m0.RecalcStyleDuration) * 1000,
      upP95: up.p95,
      upMax: up.max,
      upLongMs: up.longMs,
      downP95: down.p95,
      downMax: down.max,
      downLongMs: down.longMs,
    };
  },
  async sidebar1500() {
    const { context, page } = await boot({ conversations: 1500 });
    await page.waitForTimeout(600);
    const r = await page.evaluate(
      `${SCROLL_FN}({ selector: (() => { let e = document.querySelector('[role="option"]'); while (e && !(e.scrollHeight > e.clientHeight + 50 && /(auto|scroll)/.test(getComputedStyle(e).overflowY))) e = e.parentElement; return e; })(), steps: 150, delta: 300 })`,
    );
    const nodes = await page.evaluate(() => document.getElementsByTagName("*").length);
    await closeRun(page, context);
    return { ...r, nodes };
  },
  async filter20k() {
    const { context, page } = await boot({ dirEntries: 20000 });
    await page.evaluate(() =>
      [...document.querySelectorAll("button.nav-destination")]
        .find((b) => b.textContent.trim().startsWith("Projects"))
        .click(),
    );
    await page.waitForSelector('[aria-label="Filter visible project files"]', { timeout: 60000 });
    await page.waitForTimeout(3000);
    const times = [];
    for (const ch of "part-1") {
      const t = await page.evaluate(async (c) => {
        const input = document.querySelector('[aria-label="Filter visible project files"]');
        const t0 = performance.now();
        const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
        setter.call(input, input.value + c);
        input.dispatchEvent(new Event("input", { bubbles: true }));
        await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
        return performance.now() - t0;
      }, ch);
      times.push(t);
    }
    await closeRun(page, context);
    return { keystrokeMs: median(times), firstKeystrokeMs: times[0], all: times.map((t) => +t.toFixed(0)) };
  },
};

async function openConversation(messages) {
  const { context, page, cdp } = await boot({ messages });
  const before = await metrics(cdp);
  const r = await page.evaluate(`(async () => {
    const t0 = performance.now();
    document.querySelector('[role="option"]').click();
    await ${WAIT_FN}(() => document.querySelector(".chat-column-messages [data-index]") !== null, "the message rows");
    const rowsAt = performance.now() - t0;
    await new Promise((res) => requestAnimationFrame(() => requestAnimationFrame(res)));
    return {
      firstRowMs: rowsAt,
      paintMs: performance.now() - t0,
      rows: document.querySelectorAll(".chat-column-messages [data-index]").length,
    };
  })()`);
  await page.waitForTimeout(300);
  const after = await metrics(cdp);
  await closeRun(page, context);
  return {
    ...r,
    scriptMs: (after.ScriptDuration - before.ScriptDuration) * 1000,
    taskMs: (after.TaskDuration - before.TaskDuration) * 1000,
  };
}

const unknownNames = only.filter((n) => !Object.hasOwn(scenarios, n));
if (unknownNames.length)
  usageError(`unknown scenario ${unknownNames.join(",")}; known: ${Object.keys(scenarios).join(",")}`);

try {
  const started = await serve(dist, port);
  server = started.server;
  base = `http://127.0.0.1:${started.port}/`;
  browser = await chromium.launch({ args: ["--no-sandbox"] });
  const out = { label, runs, throttle, results: {} };
  const samplesByName = Object.fromEntries(only.map((name) => [name, []]));
  // Pass 0 is a discarded warm-up over every selected scenario; passes 1..runs are measured.
  for (let i = 0; i < runs + 1; i++) {
    for (const name of only) {
      const sample = await scenarios[name]();
      if (i > 0) samplesByName[name].push(sample);
    }
  }
  for (const name of only) {
    const samples = samplesByName[name];
    const agg = {};
    for (const k of Object.keys(samples[0]))
      if (typeof samples[0][k] === "number") agg[k] = +median(samples.map((s) => s[k])).toFixed(1);
    if (Object.keys(agg).length === 0) throw new Error(`${name} returned no numeric results`);
    out.results[name] = agg;
    if (samples[0].all) out.results[name].sample = samples[0].all;
  }
  out.unknownCommands = [...unknownSeen].sort();
  console.log(JSON.stringify(out, null, 2));
  if (json) writeFileSync(json, JSON.stringify({ ...out, samples: samplesByName }, null, 2));
} catch (e) {
  console.error(`scenarios failed: ${e.message}`);
  process.exitCode = 1;
} finally {
  await browser?.close();
  server?.close();
}
