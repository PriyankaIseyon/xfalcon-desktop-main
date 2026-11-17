import { readFileSync } from "node:fs";
import { expect, test } from "@playwright/test";
import type { ReportView } from "../../src/lib/generated/contracts";
import { installDatasetReportsMock, ok } from "./datasets-reports-native-mock";

// Exercise the real iframe consumers with the backend's policy, rather than
// quietly allowing scripts in the browser harness that production blocks.
const protocol = readFileSync("src-tauri/src/preview/protocol.rs", "utf8");
function originSources(name: string): string {
  const value = protocol.match(new RegExp(`macro_rules! ${name} \\{[\\s\\S]*?"([^"]+)"`))?.[1];
  if (!value) throw new Error(`Missing preview policy macro: ${name}`);
  return value;
}
const policyBody = protocol.match(/const HTML_DOCUMENT_CSP: &str = concat!\(([\s\S]*?)\);/)?.[1];
if (!policyBody) throw new Error("Missing HTML preview policy");
const policy = policyBody
  .replace(/preview_origin_sources!\(\)/g, JSON.stringify(originSources("preview_origin_sources")))
  .replace(/app_origin_sources!\(\)/g, JSON.stringify(originSources("app_origin_sources")))
  .match(/"[^"]*"/g)!
  .map((part) => JSON.parse(part) as string)
  .join("");
const baseStyleBody = protocol.match(/const BASE_STYLE: &str = concat!\(([\s\S]*?)\n\);/)?.[1];
if (!baseStyleBody) throw new Error("Missing preview base style");
const baseStyle = baseStyleBody
  .match(/"(?:\\.|[^"\\])*"/g)!
  .map((part) => JSON.parse(part) as string)
  .join("");

// Static KPI text alone must not satisfy this regression: the inline script
// creates every chart and mark after the sandboxed document loads.
const html = `<!doctype html><html><head><title>Compensation dashboard</title>
<style>body{font-family:sans-serif;margin:16px}main{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px}svg{display:block;width:100%;height:120px}h2{font-size:14px}</style></head><body>
<p id="kpi">Employees: 125</p><main id="charts"></main><script>
const charts=["Salary by department","Compensation by level","Salary distribution","Bonus by department","Compensation by location","Salary versus experience"];
charts.forEach((title,index)=>{
 const panel=document.createElement("section");const heading=document.createElement("h2");heading.textContent=title;panel.append(heading);
 const svg=document.createElementNS("http://www.w3.org/2000/svg","svg");svg.setAttribute("viewBox","0 0 240 120");svg.setAttribute("role","img");svg.setAttribute("aria-label",title);
 [30,65,90].forEach((value,i)=>{const mark=document.createElementNS(svg.namespaceURI,index===5?"circle":"rect");if(index===5){mark.setAttribute("cx",String(35+i*70));mark.setAttribute("cy",String(110-value));mark.setAttribute("r","6");}else{mark.setAttribute("x",String(10+i*70));mark.setAttribute("y",String(110-value));mark.setAttribute("width","45");mark.setAttribute("height",String(value));}mark.setAttribute("fill","#2363b9");svg.append(mark);});panel.append(svg);document.getElementById("charts").append(panel);
});
</script></body></html>`;

test("report viewer and HTML canvas render every inline chart, including after reload", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 });
  const report: ReportView = {
    version: 1,
    id: "chart-preview",
    title: "Compensation dashboard",
    description: null,
    created_at_ms: 1 as unknown as bigint,
    updated_at_ms: 1 as unknown as bigint,
    thumbnail_updated_at_ms: null,
    data_sources: [],
    pinned: false,
    hidden: false,
    kind: "dashboard",
    byte_size: html.length as unknown as bigint,
    scope: "user",
    is_owner: true,
    owner_display_name: null,
    refreshable: true,
    can_refresh: true,
    can_manage: true,
    can_delete: true,
    page_count: 1,
    dataset_count: 1,
    source_conversation_id: null,
  };
  await installDatasetReportsMock(page, {
    reports: [ok({ version: 1, reports: [report], project_origins: [] })],
    reportOpen: [
      ok({
        version: 1,
        id: report.id,
        title: report.title,
        description: null,
        page: "index.html",
        pages: ["index.html"],
        document_id: "saved-document",
        datasets: [],
      }),
    ],
  });
  await page.route("http://xf-preview.localhost/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "text/html; charset=utf-8",
      headers: { "Content-Security-Policy": policy },
      body: html.replace("<head>", `<head>${baseStyle}`),
    }),
  );
  await page.goto("/");
  await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();
  await page.getByRole("button", { name: "Reports", exact: true }).click();
  await page.getByRole("button", { name: /Open Compensation dashboard/ }).click();
  const assertCharts = async () => {
    const iframe = page.locator('iframe[title="Compensation dashboard"]').last();
    await expect(iframe).toHaveAttribute("sandbox", "allow-scripts");
    await expect(iframe).not.toHaveAttribute("srcdoc");
    await expect(iframe).toHaveCSS("opacity", "1");
    const frame = iframe.contentFrame();
    await expect(frame.locator("html")).toHaveCSS("scrollbar-width", "thin");
    await expect(frame.locator("#kpi")).toHaveText("Employees: 125");
    await expect(frame.locator("svg")).toHaveCount(6);
    await expect(frame.locator("svg rect, svg circle")).toHaveCount(18);
    for (const chart of await frame.locator("svg").all()) await expect(chart).toBeVisible();
  };
  await assertCharts();
  await page.getByRole("button", { name: "Back to reports" }).click();
  await page.evaluate(async (content) => {
    const modulePath = "/src/features/canvas/canvasStore.ts";
    const store = await import(/* @vite-ignore */ modulePath);
    store.openCanvasArtifact({
      id: "canvas-charts",
      type: "html",
      kind: "html",
      title: "Compensation dashboard",
      documentId: "stale-document",
      content,
    });
  }, html);
  await assertCharts();
  await expect(page.locator('iframe[title="Compensation dashboard"]').last()).toHaveAttribute(
    "src",
    "http://xf-preview.localhost/chat-preview-document",
  );
  await page.getByRole("button", { name: "Reload", exact: true }).click();
  await assertCharts();
  for (const width of [960, 1440, 1920]) {
    await page.setViewportSize({ width, height: 720 });
    const iframe = page.locator('iframe[title="Compensation dashboard"]').last();
    await iframe
      .contentFrame()
      .locator("body")
      .evaluate((node) => {
        node.style.minHeight = "1600px";
      });
    // The document owns its scroll; its ancestors must fit the workspace.
    const overflowingParents = await iframe.evaluate((node) => {
      const overflow: string[] = [];
      for (let parent = node.parentElement; parent; parent = parent.parentElement) {
        if (parent.scrollHeight > parent.clientHeight + 1 || parent.scrollWidth > parent.clientWidth + 1)
          overflow.push(parent.className);
      }
      return overflow;
    });
    expect(overflowingParents).toEqual([]);
  }
});
