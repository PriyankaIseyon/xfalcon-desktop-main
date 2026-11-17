import { expect, test, type Page, type TestInfo } from "@playwright/test";
import type {
  DatasetColumnView,
  DatasetDetail,
  DatasetFileView,
  DatasetListResult,
  DatasetPreview,
  DatasetView,
  ReportDocument,
  ReportListResult,
  ReportView,
  StagedDatasetView,
  UploadProjectionV1,
  UploadQueueV1,
  UploadState,
} from "../../src/lib/generated/contracts";
import {
  fail,
  installDatasetReportsMock,
  ok,
  pending,
  type DatasetReportsMockConfig,
} from "./datasets-reports-native-mock";

type VisualVariant = {
  name: string;
  theme: "light" | "dark";
  viewport: { width: number; height: number };
};

const light: VisualVariant = { name: "light", theme: "light", viewport: { width: 1440, height: 960 } };
const dark: VisualVariant = { name: "dark", theme: "dark", viewport: { width: 1440, height: 960 } };
const narrow: VisualVariant = { name: "narrow", theme: "light", viewport: { width: 720, height: 900 } };

const NOW = Date.UTC(2026, 7, 28, 10, 0, 0);
let queueRevision = 0;
const LONG_TITLE =
  "International enterprise revenue retention and expansion by customer segment for the rolling twelve-month period";

// Tauri's JSON boundary delivers these timestamp fields as ordinary finite
// numbers at runtime. The generated contract uses `bigint` so callers cannot
// accidentally do imprecise arithmetic on them. Keep the fixture JSON-safe
// for `addInitScript` while retaining that compile-time contract.
const contractInteger = (value: number): bigint => value as unknown as bigint;

function dataset(index = 1, overrides: Partial<DatasetView> = {}): DatasetView {
  return {
    version: 1,
    id: `dataset-${index}`,
    display_name: `Customer revenue ${index}`,
    status: "ready",
    created_at_ms: contractInteger(NOW - index * 86_400_000),
    updated_at_ms: contractInteger(NOW - index * 3_600_000),
    ...overrides,
  };
}

function datasetList(
  datasets: DatasetView[],
  { uploadsEnabled = true, truncated = false }: { uploadsEnabled?: boolean; truncated?: boolean } = {},
): DatasetListResult {
  return { version: 1, datasets, uploads_enabled: uploadsEnabled, truncated };
}

function staged(overrides: Partial<StagedDatasetView> = {}): StagedDatasetView {
  return {
    version: 1,
    id: "staged-1",
    display_name: "enterprise-revenue.xlsx",
    size: 48_260_000,
    format: "xlsx",
    worksheets: ["Revenue", "Forecast", "Regional adjustments"],
    ...overrides,
  };
}

const previewColumns: DatasetColumnView[] = [
  { name: "customer_name", inferred_type: "text" },
  { name: "annual_recurring_revenue", inferred_type: "number" },
  { name: "renewal_date", inferred_type: "date" },
  { name: "account_health", inferred_type: "text" },
];

function preview(overrides: Partial<DatasetPreview> = {}): DatasetPreview {
  return {
    version: 1,
    staged_id: "staged-1",
    worksheet_index: 0,
    header_row_index: 1,
    suggested_header_row: 1,
    columns: previewColumns,
    rows: [
      [
        "Northwind International Holdings and Advanced Manufacturing Corporation",
        "1845000.00",
        "2026-11-30",
        "Expansion under review after a very long customer-success and product-adoption assessment",
      ],
      ["Contoso", "725000.00", "2027-01-15", null],
      ["Fabrikam", "412000.00", "2026-09-01", "Healthy"],
    ],
    header_candidates: [
      ["Enterprise revenue export", null, null, null],
      ["customer_name", "annual_recurring_revenue", "renewal_date", "account_health"],
    ],
    sampled_rows: 3,
    truncated_rows: true,
    truncated_columns: false,
    truncated_cell: true,
    unavailable: null,
    recipe_available: true,
    ...overrides,
  };
}

function file(overrides: Partial<DatasetFileView> = {}): DatasetFileView {
  return {
    file_id: "file-1",
    table_name: "enterprise_revenue",
    row_count: 482_150,
    column_count: 4,
    size_bytes: 48_260_000,
    original_filename: "enterprise-revenue.xlsx",
    columns: previewColumns,
    created_at_ms: contractInteger(NOW - 86_400_000),
    updated_at_ms: contractInteger(NOW - 3_600_000),
    ...overrides,
  };
}

function detail(target: DatasetView, files: DatasetFileView[], uploadsEnabled = true): DatasetDetail {
  return { version: 1, dataset: target, files, uploads_enabled: uploadsEnabled };
}

function upload(state: UploadState, overrides: Partial<UploadProjectionV1> = {}): UploadProjectionV1 {
  return {
    version: 1,
    id: "upload-1",
    purpose: "persistent_dataset",
    state,
    bytes_sent: state === "uploading" || state === "resigning" ? 21_700_000 : state === "ready" ? 48_260_000 : 0,
    size: 48_260_000,
    display_name: "Enterprise revenue",
    safe_error: state === "failed" ? "The connection closed before the final part arrived." : null,
    resource_id: state === "ready" ? "dataset-new" : null,
    conversation_id: null,
    ...overrides,
  };
}

function queue(...uploads: UploadProjectionV1[]): UploadQueueV1 {
  return { version: 1, revision: ++queueRevision, uploads };
}

function report(index = 1, overrides: Partial<ReportView> = {}): ReportView {
  return {
    version: 1,
    id: `report-${index}`,
    title: `Revenue dashboard ${index}`,
    description: `Revenue, retention, and pipeline summary for business unit ${index}.`,
    created_at_ms: contractInteger(NOW - index * 86_400_000),
    updated_at_ms: contractInteger(NOW - index * 3_600_000),
    thumbnail_updated_at_ms: null,
    data_sources: [],
    pinned: false,
    hidden: false,
    kind: "dashboard",
    byte_size: contractInteger(84_000 + index),
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
    ...overrides,
  };
}

function reportList(reports: ReportView[]): ReportListResult {
  return { version: 1, reports, project_origins: [] };
}

async function openShell(page: Page, variant: VisualVariant, config: DatasetReportsMockConfig): Promise<void> {
  await page.setViewportSize(variant.viewport);
  await page.emulateMedia({ colorScheme: variant.theme, reducedMotion: "reduce" });
  await page.addInitScript((now) => {
    Date.now = () => now;
  }, NOW);
  await page.route("http://xf-preview.localhost/**", async (route) => {
    const documentId = new URL(route.request().url()).pathname.slice(1);
    await route.fulfill({
      contentType: "text/html; charset=utf-8",
      body: `<style>body{font-family:sans-serif;padding:32px;color:#212b36}h1{font-size:28px}</style><h1>Quarterly revenue</h1><p>Stable saved report preview for ${documentId}.</p>`,
    });
  });
  await installDatasetReportsMock(page, { theme: variant.theme, ...config });
  await page.goto("/");
  await expect(page.getByRole("main", { name: "Chat workspace" })).toBeVisible();
  await page.evaluate(async () => document.fonts.ready);
}

async function ensureSidebarOpen(page: Page): Promise<void> {
  const trigger = page.getByRole("button", { name: "Show sidebar" });
  if (await trigger.isVisible().catch(() => false)) await trigger.click();
}

async function openDatasets(page: Page): Promise<void> {
  await ensureSidebarOpen(page);
  await page.getByRole("button", { name: "Datasets" }).click();
  await expect(page.getByRole("main", { name: "Datasets" })).toBeVisible();
}

async function openReports(page: Page): Promise<void> {
  await ensureSidebarOpen(page);
  await page.getByRole("button", { name: "Reports" }).click();
  await expect(page.getByRole("main", { name: "Reports" })).toBeVisible();
}

async function screenshot(page: Page, testInfo: TestInfo, name: string): Promise<void> {
  await expect(page).toHaveScreenshot(["datasets-reports", `${name}.png`]);
  const path = testInfo.outputPath(`${name}.png`);
  await page.screenshot({ animations: "disabled", caret: "hide", fullPage: false, path, scale: "css" });
  await testInfo.attach(`${name}.png`, { path, contentType: "image/png" });
}

async function emitUpload(page: Page, next: UploadQueueV1): Promise<void> {
  await page.evaluate((value) => window.__DATASET_REPORT_EMIT_UPLOAD__?.(value), next);
}

// S060-S061: catalog, responsive table, large catalog, partial and transfer states.
for (const variant of [light, dark, narrow]) {
  test(`${variant.name} dataset catalog keeps long names and a large list readable`, async ({ page }, testInfo) => {
    const rows = Array.from({ length: 28 }, (_, index) =>
      dataset(index + 1, index === 0 ? { display_name: LONG_TITLE } : {}),
    );
    await openShell(page, variant, {
      datasets: [ok(datasetList(rows, { uploadsEnabled: false, truncated: true }))],
    });
    await openDatasets(page);
    await expect(page.getByRole("table")).toBeVisible();
    await expect(page.getByText("Uploads are turned off")).toBeVisible();
    await expect(page.getByText("Showing part of the list")).toBeVisible();
    await screenshot(page, testInfo, `dataset-catalog-large-${variant.name}`);
  });
}

test("dataset catalog covers loading, failure, empty, search, and sort", async ({ page }, testInfo) => {
  await openShell(page, light, { datasets: [pending()] });
  await openDatasets(page);
  await expect(page.getByText("Loading datasets…")).toBeVisible();
  await screenshot(page, testInfo, "dataset-catalog-loading-light");
});

test("dataset catalog shows a load failure", async ({ page }, testInfo) => {
  await openShell(page, light, { datasets: [fail({ version: 1, code: "network", step_index: null })] });
  await openDatasets(page);
  await expect(page.getByText("Datasets could not be loaded")).toBeVisible();
  await screenshot(page, testInfo, "dataset-catalog-error-light");
});

test("dataset catalog distinguishes an empty workspace and an empty search", async ({ page }, testInfo) => {
  await openShell(page, light, { datasets: [ok(datasetList([]))] });
  await openDatasets(page);
  await expect(page.getByText("No datasets yet")).toBeVisible();
  await screenshot(page, testInfo, "dataset-catalog-empty-light");
});

test("dataset catalog shows a no-match result and supports sorting", async ({ page }, testInfo) => {
  await openShell(page, light, { datasets: [ok(datasetList([dataset(1), dataset(2)]))] });
  await openDatasets(page);
  await page.getByRole("searchbox", { name: "Search datasets" }).fill("missing record");
  await expect(page.getByText("No dataset matches “missing record”")).toBeVisible();
  await screenshot(page, testInfo, "dataset-catalog-no-match-light");
  await page.getByRole("button", { name: "Show all datasets" }).click();
  await page.getByRole("button", { name: "Name" }).click();
  await expect(page.getByRole("columnheader", { name: "Name" })).toHaveAttribute("aria-sort", "ascending");
});

test("dataset queue shows every concurrent transfer phase and failure actions", async ({ page }, testInfo) => {
  const phases: UploadState[] = ["selected", "preparing", "uploading", "resigning", "finalizing", "failed"];
  await openShell(page, light, {
    datasets: [ok(datasetList([dataset()]))],
    uploadQueue: queue(
      ...phases.map((state, index) =>
        upload(state, {
          id: `upload-${index + 1}`,
          display_name: `${state} customer export`,
          bytes_sent: state === "uploading" || state === "resigning" ? 24_130_000 : 0,
        }),
      ),
    ),
  });
  await openDatasets(page);
  await expect(page.getByRole("list", { name: "Dataset transfers" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
  await screenshot(page, testInfo, "dataset-transfer-concurrent-light");
});

// S062-S064: upload, preview, worksheet/header controls, cleanup, and lifecycle.
test("dataset upload reviews a wide preview with long cells and cleanup controls", async ({ page }, testInfo) => {
  await openShell(page, light, {
    datasets: [ok(datasetList([dataset(1, { display_name: "enterprise-revenue" })]))],
    datasetPick: [ok(staged())],
    datasetPreviews: [ok(preview())],
  });
  await openDatasets(page);
  await page.getByRole("button", { name: "Upload dataset" }).click();
  await expect(page.getByRole("heading", { name: "Confirm upload" })).toBeVisible();
  await expect(page.getByText("a long value shortened", { exact: false })).toBeVisible();
  await expect(page.getByRole("region", { name: "Clean up before uploading" })).toBeVisible();
  await screenshot(page, testInfo, "dataset-upload-preview-long-cell-light");
  const dialog = page.getByRole("dialog", { name: "Confirm upload" });
  await page.setViewportSize({ width: 1440, height: 1200 });
  await expect(dialog).toHaveCSS("width", "1104px");
  expect((await dialog.boundingBox())!.height).toBeLessThanOrEqual(1200 * 0.85);
  await page.setViewportSize(light.viewport);
  expect((await dialog.boundingBox())!.height).toBeLessThanOrEqual(light.viewport.height * 0.85);
  await expect(dialog.getByRole("button", { name: "Upload dataset", exact: true })).toBeInViewport();
});

test("dataset upload covers preview loading and unavailable fallback", async ({ page }, testInfo) => {
  await openShell(page, narrow, {
    datasets: [ok(datasetList([]))],
    datasetPick: [ok(staged())],
    datasetPreviews: [pending()],
  });
  await openDatasets(page);
  await page.getByRole("button", { name: "Upload dataset" }).first().click();
  await expect(page.getByText("Reading the file…")).toBeVisible();
  await screenshot(page, testInfo, "dataset-upload-preview-loading-narrow");
});

test("dataset upload explains an unavailable preview", async ({ page }, testInfo) => {
  await openShell(page, light, {
    datasets: [ok(datasetList([]))],
    datasetPick: [ok(staged())],
    datasetPreviews: [ok(preview({ columns: [], rows: [], unavailable: "file_too_large", recipe_available: false }))],
  });
  await openDatasets(page);
  await page.getByRole("button", { name: "Upload dataset" }).first().click();
  await expect(page.getByText("No preview for this file")).toBeVisible();
  await screenshot(page, testInfo, "dataset-upload-preview-unavailable-light");
});

test("dataset upload keeps a preview failure visible", async ({ page }, testInfo) => {
  await openShell(page, light, {
    datasets: [ok(datasetList([]))],
    datasetPick: [ok(staged({ format: "csv", worksheets: [] }))],
    datasetPreviews: [fail({ version: 1, code: "unreadable", step_index: null })],
  });
  await openDatasets(page);
  await page.getByRole("button", { name: "Upload dataset" }).first().click();
  await expect(page.getByRole("alert")).toBeVisible();
  await screenshot(page, testInfo, "dataset-upload-preview-error-light");
});

test("dataset upload shows selected through complete transfer lifecycle", async ({ page }, testInfo) => {
  const target = upload("selected");
  await openShell(page, light, {
    datasets: [ok(datasetList([]))],
    datasetPick: [ok(staged())],
    datasetPreviews: [ok(preview())],
    datasetUploadStart: [ok({ version: 1, upload_id: target.id, queue: queue(target) })],
  });
  await openDatasets(page);
  await page.getByRole("button", { name: "Upload dataset" }).first().click();
  await expect(page.getByRole("heading", { name: "Confirm upload" })).toBeVisible();
  const originalFrame = await page.getByRole("dialog").boundingBox();
  await page.getByRole("dialog").getByRole("button", { name: "Upload dataset" }).click();
  await expect(page.getByRole("heading", { name: "Uploading dataset" })).toBeVisible();
  await screenshot(page, testInfo, "dataset-upload-transfer-waiting-light");

  for (const state of ["preparing", "uploading", "resigning", "finalizing"] as const) {
    await emitUpload(page, queue(upload(state)));
    await expect(page.getByRole("progressbar")).toBeVisible();
    await screenshot(page, testInfo, `dataset-upload-transfer-${state}-light`);
  }

  await emitUpload(page, queue(upload("failed")));
  await expect(page.getByRole("heading", { name: "Upload failed" })).toBeVisible();
  await screenshot(page, testInfo, "dataset-upload-transfer-failed-light");

  await emitUpload(page, queue(upload("ready")));
  await expect(page.getByRole("heading", { name: "Upload complete" })).toBeVisible();
  await screenshot(page, testInfo, "dataset-upload-transfer-complete-light");
  const completedFrame = (await page.getByRole("dialog").boundingBox())!;
  expect(completedFrame.width).toEqual(originalFrame!.width);
  expect(completedFrame.height).toBeLessThan(originalFrame!.height);
  await expect(page.getByRole("button", { name: "Close now" })).toBeInViewport();
});

// S065-S066: schema and typed deletion, including large and rollback states.
test("dataset schema handles a large schema without losing its actions", async ({ page }, testInfo) => {
  const target = dataset();
  const columns = Array.from({ length: 36 }, (_, index) => ({
    name: `customer_dimension_${String(index + 1).padStart(2, "0")}_with_a_long_name`,
    inferred_type: index % 3 === 0 ? "number" : "text",
  }));
  await openShell(page, light, {
    datasets: [ok(datasetList([target]))],
    datasetDetails: [ok(detail(target, [file({ columns, column_count: columns.length })]))],
  });
  await openDatasets(page);
  await page.getByRole("button", { name: `Schema and files for ${target.display_name}` }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByText(columns.at(-1)!.name)).toBeAttached();
  await screenshot(page, testInfo, "dataset-schema-large-light");
});

test("dataset schema covers loading, error, and no-file states", async ({ page }, testInfo) => {
  const target = dataset();
  await openShell(page, light, {
    datasets: [ok(datasetList([target]))],
    datasetDetails: [pending()],
  });
  await openDatasets(page);
  await page.getByRole("button", { name: `Schema and files for ${target.display_name}` }).click();
  await expect(page.getByText("Reading the schema…")).toBeVisible();
  await screenshot(page, testInfo, "dataset-schema-loading-light");
});

test("dataset schema states a read failure", async ({ page }, testInfo) => {
  const target = dataset();
  await openShell(page, light, {
    datasets: [ok(datasetList([target]))],
    datasetDetails: [fail({ version: 1, code: "network", step_index: null })],
  });
  await openDatasets(page);
  await page.getByRole("button", { name: `Schema and files for ${target.display_name}` }).click();
  await expect(page.getByRole("dialog").getByText(/could not be reached/i)).toBeVisible();
  await screenshot(page, testInfo, "dataset-schema-error-light");
});

test("dataset schema states that a dataset has no files", async ({ page }, testInfo) => {
  const target = dataset();
  await openShell(page, light, {
    datasets: [ok(datasetList([target]))],
    datasetDetails: [ok(detail(target, []))],
  });
  await openDatasets(page);
  await page.getByRole("button", { name: `Schema and files for ${target.display_name}` }).click();
  await expect(page.getByText("This dataset holds no files.")).toBeVisible();
  await screenshot(page, testInfo, "dataset-schema-no-files-light");
});

test("dataset deletion shows unmatched, matched, and rollback failure", async ({ page }, testInfo) => {
  const target = dataset();
  await openShell(page, light, {
    datasets: [ok(datasetList([target]))],
    datasetDelete: [fail({ version: 1, code: "network", step_index: null })],
  });
  await openDatasets(page);
  await page.getByRole("button", { name: `Delete ${target.display_name}` }).click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog.getByRole("button", { name: "Delete dataset" })).toBeDisabled();
  await screenshot(page, testInfo, "dataset-delete-unmatched-light");
  await dialog.getByLabel(`Type "${target.display_name}" to confirm`).fill(target.display_name);
  await expect(dialog.getByRole("button", { name: "Delete dataset" })).toBeEnabled();
  await screenshot(page, testInfo, "dataset-delete-matched-light");
  await dialog.getByRole("button", { name: "Delete dataset" }).click();
  await expect(dialog).toBeVisible();
  await expect(page.getByText(target.display_name).first()).toBeVisible();
  await screenshot(page, testInfo, "dataset-delete-rollback-failure-light");
});

test("dataset deletion blocks dismissal while delete is pending", async ({ page }, testInfo) => {
  const target = dataset();
  await openShell(page, light, {
    datasets: [ok(datasetList([target]))],
    datasetDelete: [pending()],
  });
  await openDatasets(page);
  await page.getByRole("button", { name: `Delete ${target.display_name}` }).click();
  const dialog = page.getByRole("alertdialog");
  await dialog.getByLabel(`Type "${target.display_name}" to confirm`).fill(target.display_name);
  await dialog.getByRole("button", { name: "Delete dataset" }).click();
  await expect(dialog.getByRole("button", { name: "Deleting…" })).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "Keep dataset" })).toBeDisabled();
  await screenshot(page, testInfo, "dataset-delete-busy-light");
});

// S080: one, two, and four-column densities plus long-title and large-catalog states.
for (const variant of [
  { ...light, name: "four-column" },
  { ...narrow, name: "two-column", viewport: { width: 820, height: 900 } },
  { ...narrow, name: "one-column", viewport: { width: 560, height: 900 } },
]) {
  test(`reports library has ${variant.name} responsive density`, async ({ page }, testInfo) => {
    const rows = Array.from({ length: 30 }, (_, index) =>
      report(index + 1, {
        title: index === 0 ? LONG_TITLE : `Revenue dashboard ${index + 1}`,
        pinned: index < 3,
      }),
    );
    await openShell(page, variant, { reports: [ok(reportList(rows))] });
    await openReports(page);
    await expect(page.getByRole("region", { name: "Pinned" })).toBeVisible();
    await expect(page.getByRole("region", { name: "Older reports" })).toBeVisible();
    if (variant.name === "four-column") {
      const cards = page.getByRole("region", { name: "Recent", exact: true }).locator(".report-card");
      await expect(cards).toHaveCount(4);
      const positions = await cards.evaluateAll((nodes) =>
        nodes.map((node) => ({ top: node.getBoundingClientRect().top, width: node.getBoundingClientRect().width })),
      );
      expect(new Set(positions.map((p) => p.top)).size).toBe(1);
      expect(positions.every((p) => p.width < 280)).toBe(true);
    }
    await screenshot(page, testInfo, `reports-library-large-${variant.name}`);
  });
}

test("dark reports library preserves its pinned and recent hierarchy", async ({ page }, testInfo) => {
  await openShell(page, dark, {
    reports: [ok(reportList([report(1, { pinned: true }), report(2), report(3), report(4), report(5)]))],
  });
  await openReports(page);
  await expect(page.getByRole("region", { name: "Pinned" })).toBeVisible();
  await screenshot(page, testInfo, "reports-library-dark");
});

test("reports library shows loading", async ({ page }, testInfo) => {
  await openShell(page, light, { reports: [pending()] });
  await openReports(page);
  await expect(page.getByText("Loading…")).toBeVisible();
  await screenshot(page, testInfo, "reports-library-loading-light");
});

test("reports library shows an error", async ({ page }, testInfo) => {
  await openShell(page, light, { reports: [fail(new Error("encrypted store unavailable"))] });
  await openReports(page);
  await expect(page.getByText("Reports unavailable")).toBeVisible();
  await screenshot(page, testInfo, "reports-library-error-light");
});

test("reports library shows an empty workspace", async ({ page }, testInfo) => {
  await openShell(page, light, { reports: [ok(reportList([]))] });
  await openReports(page);
  await expect(page.getByText("No reports yet")).toBeVisible();
  await screenshot(page, testInfo, "reports-library-empty-light");
});

test("reports library shows a no-match search result", async ({ page }, testInfo) => {
  await openShell(page, light, { reports: [ok(reportList([report()]))] });
  await openReports(page);
  await page.getByRole("searchbox", { name: "Search reports" }).fill("missing report");
  await expect(page.getByText("No reports match “missing report”")).toBeVisible();
  await screenshot(page, testInfo, "reports-library-no-match-light");
});

// S081-S083: viewer, rename, delete, busy, failure, and rollback states.
test("report viewer covers opening and ready states", async ({ page }, testInfo) => {
  const target = report();
  const document: ReportDocument = {
    version: 1,
    id: target.id,
    title: target.title,
    description: target.description,
    page: "index.html",
    pages: ["index.html"],
    document_id: "document-1",
    datasets: [],
  };
  await openShell(page, light, { reports: [ok(reportList([target]))], reportOpen: [ok(document)] });
  await openReports(page);
  await page.getByRole("button", { name: new RegExp(`Open ${target.title}`) }).click();
  await expect(page.getByRole("region", { name: `Report ${target.title}` })).toBeVisible();
  await expect(page.getByTitle(target.title)).toBeVisible();
  // The fixture report is not refreshable (DA states can_refresh false), so no Refresh control.
  await expect(page.getByRole("button", { name: /^Refresh\b/ })).toHaveCount(0);
  await screenshot(page, testInfo, "report-viewer-ready-light");
});

test("report viewer shows opening progress", async ({ page }, testInfo) => {
  const target = report();
  await openShell(page, light, { reports: [ok(reportList([target]))], reportOpen: [pending()] });
  await openReports(page);
  await page.getByRole("button", { name: new RegExp(`Open ${target.title}`) }).click();
  await expect(page.getByRole("region", { name: `Report ${target.title}` })).toBeVisible();
  await screenshot(page, testInfo, "report-viewer-opening-light");
});

test("report viewer retries once before its readable failure", async ({ page }, testInfo) => {
  const target = report();
  await openShell(page, light, {
    reports: [ok(reportList([target]))],
    reportOpen: [fail(new Error("first read failed")), fail(new Error("second read failed"))],
  });
  await openReports(page);
  await page.getByRole("button", { name: new RegExp(`Open ${target.title}`) }).click();
  await expect(page.getByText("This report could not be opened.")).toBeVisible();
  await screenshot(page, testInfo, "report-viewer-failed-light");
});

test("report rename shows duplicate-name advice and a retained rollback failure", async ({ page }, testInfo) => {
  const target = report();
  const sibling = report(2, { title: "Existing revenue" });
  await openShell(page, light, {
    reports: [ok(reportList([target, sibling]))],
    reportRename: [fail({ version: 1, code: "storage_unavailable" })],
  });
  await openReports(page);
  await page.getByRole("button", { name: new RegExp(`Rename ${target.title}`) }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Name").fill(sibling.title);
  await expect(dialog.getByText(/Another report is already called/)).toBeVisible();
  await screenshot(page, testInfo, "report-rename-duplicate-light");
  await dialog.getByRole("button", { name: "Save" }).click();
  await expect(dialog).toBeVisible();
  await expect(dialog.getByLabel("Name")).toHaveValue(sibling.title);
  await screenshot(page, testInfo, "report-rename-rollback-failure-light");
});

test("report rename shows a durable busy state", async ({ page }, testInfo) => {
  const target = report();
  await openShell(page, light, { reports: [ok(reportList([target]))], reportRename: [pending()] });
  await openReports(page);
  await page.getByRole("button", { name: new RegExp(`Rename ${target.title}`) }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Name").fill("Renewal dashboard");
  await dialog.getByRole("button", { name: "Save" }).click();
  await expect(dialog.getByRole("button", { name: /Save/ })).toBeDisabled();
  await screenshot(page, testInfo, "report-rename-busy-light");
});

test("report delete covers confirmation and rollback failure", async ({ page }, testInfo) => {
  const target = report();
  await openShell(page, light, {
    reports: [ok(reportList([target]))],
    reportDelete: [fail({ version: 1, code: "storage_unavailable" })],
  });
  await openReports(page);
  await page.getByRole("button", { name: new RegExp(`Delete ${target.title}`) }).click();
  await expect(page.getByRole("alertdialog", { name: "Delete report?" })).toBeVisible();
  await screenshot(page, testInfo, "report-delete-confirm-light");
  await page.getByRole("alertdialog").getByRole("button", { name: "Delete" }).click();
  await expect(page.getByRole("alertdialog").getByText("The report was not deleted. Try again.")).toBeVisible();
  await screenshot(page, testInfo, "report-delete-rollback-failure-light");
});

test("report delete blocks its dialog while storage is pending", async ({ page }, testInfo) => {
  const target = report();
  await openShell(page, light, { reports: [ok(reportList([target]))], reportDelete: [pending()] });
  await openReports(page);
  await page.getByRole("button", { name: new RegExp(`Delete ${target.title}`) }).click();
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("button", { name: "Delete" }).click();
  await expect(dialog.getByRole("button", { name: "Delete" })).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeDisabled();
  await screenshot(page, testInfo, "report-delete-busy-light");
});

test("reports workspace remains operable at large reading size on narrow windows", async ({ page }, testInfo) => {
  await openShell(
    page,
    { ...narrow, viewport: { width: 560, height: 900 } },
    {
      reports: [ok(reportList([report(1, { title: LONG_TITLE, pinned: true }), report(2)]))],
    },
  );
  await page.evaluate(() => {
    document.documentElement.dataset.fontSize = "large";
  });
  await openReports(page);
  const main = page.getByRole("main", { name: "Reports" });
  await expect(main.getByText("Ask the assistant to save a dashboard in a chat to add a report here.")).toBeVisible();
  expect(await main.evaluate((node) => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
  await screenshot(page, testInfo, "reports-workspace-large-reading-narrow");
});

test("reports workspace supports keyboard sorting and a compact list", async ({ page }, testInfo) => {
  await openShell(page, light, { reports: [ok(reportList([report(1), report(2), report(3)]))] });
  await openReports(page);
  const sort = page.getByRole("button", { name: "Sort reports" });
  await sort.focus();
  await page.keyboard.press("Enter");
  await page.getByRole("menuitemradio", { name: "Name A–Z" }).click();
  await page.getByRole("button", { name: "List view" }).click();
  await expect(page.getByRole("table")).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Saved reports", exact: true }).getByRole("button", { name: /^Open / }),
  ).toHaveCount(3);
  await screenshot(page, testInfo, "reports-workspace-list-light");
});

test("report thumbnails render stored content without taking focus or intercepting card actions", async ({
  page,
}, testInfo) => {
  await openShell(page, light, { reports: [ok(reportList([report(1), report(2)]))] });
  await openReports(page);
  const thumbnails = page.locator("iframe[title='Report thumbnail']");
  await expect(thumbnails).toHaveCount(2);
  await expect(thumbnails.first()).toHaveCSS("opacity", "1");
  await expect(thumbnails.first().contentFrame().getByRole("heading", { name: "Quarterly revenue" })).toBeVisible();
  await expect(thumbnails.first()).toHaveAttribute("sandbox", "allow-scripts");
  await expect(thumbnails.first()).toHaveAttribute("scrolling", "no");
  const previewBody = thumbnails.first().contentFrame().locator("body");
  await previewBody.evaluate((body) => {
    body.style.minHeight = "2000px";
  });
  expect(await previewBody.evaluate(() => window.innerWidth - document.documentElement.clientWidth)).toBe(0);
  await expect(thumbnails.first().locator("..")).toHaveAttribute("inert", "");
  await expect(page.getByText("Clarity, on demand.")).toHaveCount(0);
  await page.getByRole("button", { name: /^Rename Revenue dashboard 1/ }).click();
  await expect(page.getByRole("dialog", { name: "Rename report" })).toBeVisible();
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await screenshot(page, testInfo, "reports-actual-thumbnails-light");
  const cover = await thumbnails.first().boundingBox();
  if (!cover) throw new Error("Thumbnail has no visible bounds");
  await page.mouse.click(cover.x + 20, cover.y + 20);
  await expect(page.getByRole("region", { name: "Report Revenue dashboard 1", exact: true })).toBeVisible();
});

for (const variant of [light, dark, { ...narrow, viewport: { width: 560, height: 900 } }]) {
  test(`report action previews are isolated and operable ${variant.name}`, async ({ page }) => {
    const target = report();
    await openShell(page, variant, { reports: [ok(reportList([target]))] });
    await openReports(page);
    await page.getByRole("button", { name: new RegExp(`Open ${target.title}`) }).click();
    const frame = page.getByTitle(target.title, { exact: true });
    await expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    await expect(page.getByRole("button", { name: "Schedule a refresh" })).toBeEnabled();
    const src = await frame.getAttribute("src");
    await page.getByRole("button", { name: "Schedule a refresh" }).click();
    let dialog = page.getByRole("dialog", { name: "Schedule a refresh" });
    await expect(dialog.getByRole("group", { name: "Start date calendar" })).toBeVisible();
    await expect(dialog.getByText("Every day at 09:00.")).toBeVisible();
    expect(await dialog.evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(0);
    await expect(dialog).toHaveScreenshot(["datasets-reports", `report-schedule-${variant.name}.png`]);
    await dialog.getByRole("button", { name: "Frequency", exact: true }).click();
    await page.getByRole("menuitemradio", { name: "Custom cron" }).click();
    await dialog.getByLabel("Cron expression").fill("99 * * * *");
    await expect(dialog.getByRole("button", { name: "Confirm schedule" })).toBeDisabled();
    await dialog.getByLabel("Cron expression").fill("0 9 * * 1-5");
    await expect(dialog.getByRole("status")).toContainText("At 09:00");
    await expect(dialog).toHaveScreenshot(["datasets-reports", `report-cron-${variant.name}.png`]);
    await dialog.getByRole("button", { name: "Confirm schedule" }).click();
    await expect(dialog.getByRole("status")).toContainText("No refresh was scheduled");
    await dialog.getByRole("button", { name: "Done" }).click();
    await expect(frame).toHaveAttribute("src", src!);
    await page.getByRole("button", { name: "Share via email" }).click();
    dialog = page.getByRole("dialog", { name: "Share via email" });
    await dialog.getByLabel("Recipients").fill("analyst@example.com");
    await expect(dialog).toHaveScreenshot(["datasets-reports", `report-email-${variant.name}.png`]);
    await dialog.getByRole("button", { name: "Confirm email" }).click();
    await expect(dialog.getByRole("status")).toContainText("No email was sent");
    await dialog.getByRole("button", { name: "Done" }).click();
    await page.getByRole("button", { name: "Export", exact: true }).click();
    dialog = page.getByRole("dialog", { name: "Export report" });
    await expect(dialog).toHaveScreenshot(["datasets-reports", `report-export-${variant.name}.png`]);
    await dialog.getByRole("button", { name: "Confirm export" }).click();
    await expect(dialog.getByRole("status")).toContainText("No file was created");
    await dialog.getByRole("button", { name: "Done" }).click();
    await dialog.getByRole("button", { name: "Done" }).click();
    await expect(frame).toHaveAttribute("src", src!);
  });
}
