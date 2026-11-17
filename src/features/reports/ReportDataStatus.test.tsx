import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import type { ReportDatasetFreshness, ReportRefreshOutcome } from "../../lib/generated/contracts";
import { ReportDataStatus } from "./ReportDataStatus";
import { refreshHeadline } from "./reportPresentation";

afterEach(cleanup);

const outcome = (overrides: Partial<ReportRefreshOutcome> & { dataset_id: string }): ReportRefreshOutcome => ({
  version: 1,
  ok: true,
  rows: 10n,
  duration_ms: 1_000n,
  data_source: "warehouse",
  error: null,
  ...overrides,
});

const dataset = (overrides: Partial<ReportDatasetFreshness> & { dataset_id: string }): ReportDatasetFreshness => ({
  version: 1,
  executed_at_ms: 1_790_848_800_000n,
  row_count: 42n,
  ...overrides,
});

describe("refreshHeadline", () => {
  it("says how many datasets ran and the rows they returned", () => {
    expect(refreshHeadline([outcome({ dataset_id: "a", rows: 11n }), outcome({ dataset_id: "b", rows: 8n })])).toEqual({
      ok: true,
      text: "Refreshed 2 datasets · 19 rows",
    });
    expect(refreshHeadline([outcome({ dataset_id: "a", rows: 1n })])).toEqual({
      ok: true,
      text: "Refreshed 1 dataset · 1 row",
    });
  });

  it("says a part failure and a whole failure differently, and is never ok", () => {
    expect(
      refreshHeadline([outcome({ dataset_id: "a" }), outcome({ dataset_id: "b", ok: false, error: "timeout" })]),
    ).toEqual({ ok: false, text: "1 of 2 datasets refreshed · 1 failed" });
    expect(refreshHeadline([outcome({ dataset_id: "a", ok: false })])).toEqual({
      ok: false,
      text: "Refresh failed for the dataset",
    });
    expect(refreshHeadline([outcome({ dataset_id: "a", ok: false }), outcome({ dataset_id: "b", ok: false })])).toEqual(
      { ok: false, text: "Refresh failed for all 2 datasets" },
    );
  });
});

describe("ReportDataStatus", () => {
  it("states the data's age and keeps the per-dataset table closed until asked", async () => {
    const user = userEvent.setup();
    render(
      <ReportDataStatus
        datasets={[
          dataset({ dataset_id: "sales" }),
          dataset({ dataset_id: "returns", executed_at_ms: null, row_count: null }),
        ]}
        refresh={{ phase: "idle" }}
        summary="Data stored 9 Oct 2026. 1 dataset has not been refreshed yet."
      />,
    );

    expect(screen.getByText(/^Data stored 9 Oct 2026/)).toBeInTheDocument();
    const toggle = screen.getByRole("button", { name: "Details" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    // Closed content is out of the tab order and the accessibility tree.
    expect(screen.getByRole("region", { name: "Report data" }).querySelector("[inert]")).not.toBeNull();

    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("region", { name: "Report data" }).querySelector("[inert]")).toBeNull();
    expect(within(screen.getByRole("row", { name: /^returns/ })).getByText("Not refreshed yet")).toBeInTheDocument();

    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
  });

  it("shows a running refresh without changing the strip's words around it", () => {
    const { container } = render(
      <ReportDataStatus
        datasets={[dataset({ dataset_id: "sales" })]}
        refresh={{ phase: "refreshing" }}
        summary="Data stored."
      />,
    );
    expect(screen.getByText("Refreshing data…")).toBeInTheDocument();
    expect(container.querySelector('[data-tone="busy"]')).not.toBeNull();
  });

  it("opens by itself when a dataset failed, and the reader can close it", async () => {
    const user = userEvent.setup();
    render(
      <ReportDataStatus
        datasets={[dataset({ dataset_id: "sales" }), dataset({ dataset_id: "churn" })]}
        refresh={{
          phase: "done",
          outcomes: [
            outcome({ dataset_id: "sales" }),
            outcome({ dataset_id: "churn", ok: false, error: "Warehouse timeout" }),
          ],
        }}
        summary="Data stored 9 Oct 2026."
      />,
    );

    expect(screen.getByText("1 of 2 datasets refreshed · 1 failed")).toBeInTheDocument();
    const toggle = screen.getByRole("button", { name: "Details" });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("alert")).toHaveTextContent("keeps its last stored rows");
    expect(within(screen.getByRole("row", { name: /^churn/ })).getByText("Warehouse timeout")).toBeInTheDocument();
    expect(
      within(screen.getByRole("row", { name: /^sales/ })).getByText("Refreshed in 1 s from warehouse"),
    ).toBeInTheDocument();

    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
  });

  it("stays closed after a clean refresh, and still states the stored age beneath it", async () => {
    const user = userEvent.setup();
    render(
      <ReportDataStatus
        datasets={[dataset({ dataset_id: "sales" })]}
        refresh={{ phase: "done", outcomes: [outcome({ dataset_id: "sales", rows: 12n })] }}
        summary="Data stored 9 Oct 2026."
      />,
    );

    expect(screen.getByText("Refreshed 1 dataset · 12 rows")).toBeInTheDocument();
    const toggle = screen.getByRole("button", { name: "Details" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    await user.click(toggle);
    expect(screen.getByText("Data stored 9 Oct 2026.")).toBeInTheDocument();
  });

  it("offers no Details when there is nothing to detail", () => {
    render(
      <ReportDataStatus datasets={[]} refresh={{ phase: "idle" }} summary="Saved data has not been refreshed yet." />,
    );
    expect(screen.queryByRole("button", { name: "Details" })).not.toBeInTheDocument();
  });

  it("never says the data is current", () => {
    render(
      <ReportDataStatus
        datasets={[dataset({ dataset_id: "sales" })]}
        refresh={{ phase: "done", outcomes: [outcome({ dataset_id: "sales" })] }}
        summary="Data stored 9 Oct 2026."
      />,
    );
    expect(screen.getByRole("region", { name: "Report data" }).textContent).not.toMatch(/up to date|current|live/i);
  });
});
