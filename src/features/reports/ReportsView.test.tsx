import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReportView } from "../../lib/generated/contracts";
import { ReportsView } from "./ReportsView";
import { ReportProjectsContext, type ReportProjectChoices } from "./reportDestination";
import {
  beginReportListRequest,
  cancelReportListRequest,
  getPendingReportId,
  getReportsSnapshot,
  requestOpenReport,
  resetReportsStore,
} from "./reportsStore";
import { getToasts, resetToastsForTests } from "../../ui/toastStore";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  convertFileSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

function report(overrides: Partial<ReportView> & { id: string; title: string }): ReportView {
  return {
    version: 1,
    kind: "dashboard",
    description: null,
    created_at_ms: 1_783_500_000_000n,
    updated_at_ms: 1_783_500_000_000n,
    thumbnail_updated_at_ms: null,
    data_sources: [],
    pinned: false,
    hidden: false,
    byte_size: 4_000n,
    scope: "user",
    is_owner: true,
    owner_display_name: null,
    refreshable: false,
    can_refresh: false,
    can_manage: true,
    can_delete: true,
    page_count: 1,
    dataset_count: 0,
    source_conversation_id: "conv-1",
    ...overrides,
  };
}

const PIPELINE = report({ id: "report-1", title: "Q3 pipeline", description: "Bookings by region" });
const CHURN = report({ id: "report-2", title: "Churn by cohort", updated_at_ms: 1_783_400_000_000n });
/** Shared by a teammate: the viewer owns neither the name nor the delete. */
const SHARED = report({
  id: "report-3",
  title: "Shared pipeline",
  scope: "global",
  is_owner: false,
  owner_display_name: "Ada Lovelace",
  can_manage: false,
  can_delete: false,
});
/** Refreshable: DA answers that this viewer may re-run it. */
const REFRESHABLE = { ...PIPELINE, refreshable: true, can_refresh: true };

/** The order reports are drawn in, across every zone. Each control names its
 * report, so the open buttons are the list — read in DOM order. */
const drawnOrder = () => screen.getAllByRole("button", { name: /^Open / }).map((button) => button.textContent);

/** The one control that opens `title`. Fails loudly if two reports share the
 * name, which is what the duplicate-name tests below use instead. */
const openButtonFor = (title: string) => screen.getByRole("button", { name: new RegExp(`^Open ${title} — `) });

function mockBoundary(handlers: Record<string, (args: Record<string, unknown>) => unknown>) {
  invokeMock.mockImplementation(async (command: string, args: Record<string, unknown>) => {
    const handler = handlers[command];
    if (!handler) throw new Error(`Unexpected invoke: ${command}`);
    return handler(args ?? {});
  });
}

const listOf = (...reports: ReportView[]) => ({ version: 1 as const, reports, project_origins: [] });

const originOfReport = (report_id: string, project_id: string, project_name: string) => ({
  version: 1 as const,
  report_id,
  project_id,
  project_name,
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

afterEach(cleanup);

beforeEach(() => {
  invokeMock.mockReset();
  resetReportsStore();
  resetToastsForTests();
});

describe("ReportsView", () => {
  it("lists saved reports, pinned first", async () => {
    mockBoundary({
      report_list: () => listOf(CHURN, { ...PIPELINE, pinned: true }),
    });
    render(<ReportsView active />);

    await screen.findByText("Q3 pipeline");
    expect(drawnOrder()).toEqual(["Q3 pipeline", "Churn by cohort"]);
    expect(within(screen.getByRole("region", { name: "Pinned" })).getByText("Q3 pipeline")).toBeInTheDocument();
    expect(screen.getByText("2 saved reports")).toBeInTheDocument();
  });

  it("cards the four most recent and tables the rest", async () => {
    const older = [4, 5].map((n) =>
      report({ id: `report-${n}`, title: `Report ${n}`, updated_at_ms: BigInt(1_783_000_000_000 - n) }),
    );
    mockBoundary({
      report_list: () =>
        listOf(
          PIPELINE,
          CHURN,
          report({ id: "report-3", title: "Report 3", updated_at_ms: 1_783_300_000_000n }),
          ...older,
        ),
    });
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    // Four in the card zone, in update order; the remainder in the table, in
    // the same order continuing.
    const recent = within(screen.getByRole("region", { name: "Recent reports" }));
    expect(recent.getAllByRole("button", { name: /^Open / }).map((b) => b.textContent)).toEqual([
      "Q3 pipeline",
      "Churn by cohort",
      "Report 3",
      "Report 4",
    ]);
    const table = within(screen.getByRole("region", { name: "Older reports" }));
    expect(table.getAllByRole("button", { name: /^Open / }).map((b) => b.textContent)).toEqual(["Report 5"]);
    // Disjoint: no report is drawn twice with two sets of controls.
    expect(drawnOrder()).toHaveLength(5);
  });

  it("says how a report is made when there are none", async () => {
    mockBoundary({ report_list: () => listOf() });
    render(<ReportsView active />);

    expect(await screen.findByText("No reports yet")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Ask the assistant to save a dashboard it built, and it will appear here. Reports are stored in xFalcon, so they open on any device you sign in to.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });

  it("offers a retry when the store could not be read", async () => {
    mockBoundary({
      report_list: () => {
        throw new Error("storage_unavailable");
      },
    });
    render(<ReportsView active />);

    expect(await screen.findByText("Reports unavailable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("says nothing matched, and offers the search back", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE, CHURN) });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    await user.type(screen.getByRole("searchbox", { name: "Search reports" }), "revenue");

    expect(await screen.findByText("No reports match “revenue”")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show all reports" }));
    expect(await screen.findByText("Q3 pipeline")).toBeInTheDocument();
  });

  it("keeps the exact list and edited fields when rename fails, then retries", async () => {
    const firstRename = deferred<ReportView>();
    let renameCalls = 0;
    mockBoundary({
      report_list: () => listOf(PIPELINE, CHURN),
      report_rename: (args) => {
        renameCalls += 1;
        if (renameCalls === 1) return firstRename.promise;
        return { ...PIPELINE, title: args.title as string, description: args.description as string | null };
      },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    const initialState = getReportsSnapshot();
    const initialOrder = drawnOrder();

    await user.click(screen.getByRole("button", { name: /^Rename Q3 pipeline/ }));
    const nameField = screen.getByRole("textbox", { name: /Name/ });
    await user.clear(nameField);
    await user.type(nameField, "Q3 bookings");
    await user.click(screen.getByRole("button", { name: "Save" }));

    const busySave = screen.getByRole("button", { name: "Save" });
    expect(busySave).toBeDisabled();
    expect(busySave).toHaveAttribute("aria-busy", "true");
    await user.click(busySave);
    expect(renameCalls).toBe(1);

    await act(async () => {
      firstRename.reject(new Error("storage_unavailable"));
      await firstRename.promise.catch(() => undefined);
    });

    expect(await screen.findByText(/kept its previous name/)).toBeInTheDocument();
    expect(nameField).toHaveValue("Q3 bookings");
    expect(getReportsSnapshot()).toEqual(initialState);
    expect(drawnOrder()).toEqual(initialOrder);

    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Q3 bookings")).toBeInTheDocument();
    expect(renameCalls).toBe(2);
  });

  it("rejects a rename response for another report without changing either row", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE, CHURN),
      report_rename: () => ({ ...CHURN, title: "Wrong row" }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    const initialState = getReportsSnapshot();

    await user.click(screen.getByRole("button", { name: /^Rename Q3 pipeline/ }));
    const nameField = screen.getByRole("textbox", { name: /Name/ });
    await user.clear(nameField);
    await user.type(nameField, "Q3 bookings");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText(/kept its previous name/)).toBeInTheDocument();
    expect(getReportsSnapshot()).toEqual(initialState);
    expect(screen.getByText("Q3 pipeline")).toBeInTheDocument();
    expect(screen.getByText("Churn by cohort")).toBeInTheDocument();
    expect(screen.queryByText("Wrong row")).not.toBeInTheDocument();
  });

  it("keeps a failed delete open with the exact list, blocks duplicates, and retries", async () => {
    const firstDelete = deferred<void>();
    let deleteCalls = 0;
    mockBoundary({
      report_list: () => listOf(PIPELINE, CHURN),
      report_delete: () => {
        deleteCalls += 1;
        return deleteCalls === 1 ? firstDelete.promise : undefined;
      },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    const initialState = getReportsSnapshot();
    const initialOrder = drawnOrder();

    await user.click(screen.getByRole("button", { name: /^Delete Q3 pipeline/ }));
    // Nothing has gone yet: it removes the only copy of the document.
    expect(screen.getByText("Q3 pipeline")).toBeInTheDocument();
    expect(await screen.findByText("Delete report?")).toBeInTheDocument();

    expect(deleteCalls).toBe(0);
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(deleteCalls).toBe(0);
    expect(getReportsSnapshot()).toEqual(initialState);
    await user.click(screen.getByRole("button", { name: /^Delete Q3 pipeline/ }));

    await user.click(screen.getByRole("button", { name: "Delete" }));
    const busyDelete = screen.getByRole("button", { name: "Delete" });
    expect(busyDelete).toBeDisabled();
    expect(busyDelete).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    await user.click(busyDelete);
    expect(deleteCalls).toBe(1);

    await act(async () => {
      firstDelete.reject(new Error("storage_unavailable"));
      await firstDelete.promise.catch(() => undefined);
    });

    expect(await screen.findByText("The report was not deleted. Try again.")).toBeInTheDocument();
    expect(screen.getByText("Delete report?")).toBeInTheDocument();
    expect(getReportsSnapshot()).toEqual(initialState);
    expect(drawnOrder()).toEqual(initialOrder);

    await user.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.queryByText("Delete report?")).not.toBeInTheDocument());
    expect(screen.queryByText("Q3 pipeline")).not.toBeInTheDocument();
    expect(screen.getByText("Churn by cohort")).toBeInTheDocument();
    expect(deleteCalls).toBe(2);
    // The Delete button went with its card; focus stays in the library.
    expect(screen.getByRole("region", { name: "Saved reports" })).toHaveFocus();
  });

  it("pins with a backend-minted time, not one the surface invented", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE, CHURN),
      report_set_pinned: () => ({ ...PIPELINE, pinned: true }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: /^Pin Q3 pipeline/ }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("report_set_pinned", { reportId: "report-1", pinned: true }),
    );
    // The intent crossed the boundary; no timestamp did.
    const [, args] = invokeMock.mock.calls.find(([command]) => command === "report_set_pinned")!;
    expect(Object.keys(args as object)).toEqual(["reportId", "pinned"]);
  });

  it("keeps keyboard focus on the report's pin control after pinning moves its card", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE, CHURN),
      report_set_pinned: () => ({ ...PIPELINE, pinned: true }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: /^Pin Q3 pipeline/ }));

    // The pinned card is drawn in another zone, so the button that was focused
    // is gone; its replacement, now an Unpin, takes the focus.
    const unpin = await screen.findByRole("button", { name: /^Unpin Q3 pipeline/ });
    await waitFor(() => expect(unpin).toHaveFocus());
  });

  it("keeps pin state exact on failure, blocks duplicate actions, and retries", async () => {
    const firstPin = deferred<ReportView>();
    let pinCalls = 0;
    mockBoundary({
      report_list: () => listOf(PIPELINE, CHURN),
      report_set_pinned: () => {
        pinCalls += 1;
        if (pinCalls === 1) return firstPin.promise;
        return { ...PIPELINE, pinned: true };
      },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    const pin = await screen.findByRole("button", { name: /^Pin Q3 pipeline/ });
    const initialState = getReportsSnapshot();
    const initialOrder = drawnOrder();

    await user.click(pin);
    const busyPin = screen.getByRole("button", { name: /^Pin Q3 pipeline/ });
    expect(busyPin).toBeDisabled();
    expect(busyPin).toHaveAttribute("aria-busy", "true");
    await user.click(busyPin);
    expect(pinCalls).toBe(1);

    await act(async () => {
      firstPin.reject(new Error("storage_unavailable"));
      await firstPin.promise.catch(() => undefined);
    });

    expect(await screen.findByText("This report could not be pinned. Try again.")).toBeInTheDocument();
    expect(getReportsSnapshot()).toEqual(initialState);
    expect(drawnOrder()).toEqual(initialOrder);

    await user.click(screen.getByRole("button", { name: /^Pin Q3 pipeline/ }));
    expect(await screen.findByRole("region", { name: "Pinned" })).toHaveTextContent("Q3 pipeline");
    expect(pinCalls).toBe(2);
  });

  it("does not let an older list completion overwrite a confirmed pin", async () => {
    const pinResult = deferred<ReportView>();
    const staleList = deferred<ReturnType<typeof listOf>>();
    let listCalls = 0;
    mockBoundary({
      report_list: () => {
        listCalls += 1;
        return listCalls === 1 ? listOf(PIPELINE, CHURN) : staleList.promise;
      },
      report_set_pinned: () => pinResult.promise,
    });
    const user = userEvent.setup();
    const { rerender } = render(<ReportsView active />);

    await user.click(await screen.findByRole("button", { name: /^Pin Q3 pipeline/ }));
    rerender(<ReportsView active={false} />);
    rerender(<ReportsView active />);
    await waitFor(() => expect(listCalls).toBe(2));

    await act(async () => pinResult.resolve({ ...PIPELINE, pinned: true }));
    expect(await screen.findByRole("region", { name: "Pinned" })).toHaveTextContent("Q3 pipeline");

    await act(async () => staleList.resolve(listOf(PIPELINE, CHURN)));
    expect(screen.getByRole("region", { name: "Pinned" })).toHaveTextContent("Q3 pipeline");
    expect(getReportsSnapshot().reports[0].pinned).toBe(true);
  });

  it("rejects a pin response for another report without changing either row", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE, CHURN),
      report_set_pinned: () => ({ ...CHURN, pinned: true }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    const pin = await screen.findByRole("button", { name: /^Pin Q3 pipeline/ });
    const initialState = getReportsSnapshot();

    await user.click(pin);

    expect(await screen.findByText("This report could not be pinned. Try again.")).toBeInTheDocument();
    expect(getReportsSnapshot()).toEqual(initialState);
    expect(screen.queryByRole("region", { name: "Pinned" })).not.toBeInTheDocument();
  });

  it("opens a report in place, from its own origin, and comes back to the list", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE),
      report_open: () => ({
        version: 1,
        id: "report-1",
        title: "Q3 pipeline",
        description: null,
        page: "index.html",
        pages: ["index.html"],
        document_id: "abc123",
        datasets: [],
      }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);

    await screen.findByText("Q3 pipeline");
    await user.click(openButtonFor("Q3 pipeline"));

    // Scoped to the viewer: a card also carries its title as a tooltip, for the
    // truncated case, so an unscoped `findByTitle` can resolve on the list that
    // is about to be unmounted.
    const viewer = await screen.findByRole("region", { name: "Report Q3 pipeline" });
    const frame = within(viewer).getByTitle("Q3 pipeline");
    expect(frame).toHaveAttribute("src", "http://xf-preview.localhost/abc123");
    // The isolation contract: scripts, never same-origin, never srcdoc.
    expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    expect(frame.getAttribute("sandbox")).not.toContain("allow-same-origin");
    expect(frame).not.toHaveAttribute("srcdoc");
    expect(frame).toHaveAttribute("tabindex", "-1");
    expect(frame).toHaveAttribute("aria-hidden", "true");
    expect(within(viewer).getByRole("status")).toHaveTextContent("Opening report…");

    fireEvent.load(frame);
    expect(frame).toHaveAttribute("tabindex", "0");
    expect(frame).toHaveAttribute("aria-hidden", "false");

    await user.click(screen.getByRole("button", { name: "Back to reports" }));
    expect(await screen.findByRole("region", { name: "Saved reports" })).toBeInTheDocument();
    expect(openButtonFor("Q3 pipeline")).toBeInTheDocument();
  });

  it("re-opens once before telling the reader a report could not be opened", async () => {
    let opens = 0;
    mockBoundary({
      report_list: () => listOf(PIPELINE),
      report_open: () => {
        opens += 1;
        // The preview store is bounded and evictable, so a miss is expected to
        // be transient. Only the second failure is worth showing.
        if (opens <= 2) throw new Error("storage_unavailable");
        return {
          version: 1,
          id: "report-1",
          title: "Q3 pipeline",
          description: null,
          page: "index.html",
          pages: ["index.html"],
          document_id: "abc123",
          datasets: [],
        };
      },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user.click(await screen.findByText("Q3 pipeline"));

    expect(await screen.findByText("This report could not be opened.")).toBeInTheDocument();
    expect(opens).toBe(2);

    await user.click(screen.getByRole("button", { name: "Try again" }));
    const reopened = await screen.findByRole("region", { name: "Report Q3 pipeline" });
    expect(within(reopened).getByTitle("Q3 pipeline")).toBeInTheDocument();
  });

  it("rejects an opened document for another report", async () => {
    let opens = 0;
    mockBoundary({
      report_list: () => listOf(PIPELINE),
      report_open: () => {
        opens += 1;
        return {
          version: 1,
          id: "report-2",
          title: "Wrong report",
          description: null,
          page: "index.html",
          pages: ["index.html"],
          document_id: "wrong",
          datasets: [],
        };
      },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);

    await user.click(await screen.findByRole("button", { name: /^Open Q3 pipeline/ }));

    expect(await screen.findByText("This report could not be opened.")).toBeInTheDocument();
    expect(opens).toBe(2);
    expect(screen.queryByTitle("Wrong report")).not.toBeInTheDocument();
  });

  it("ignores a stale open when another report becomes the viewer", async () => {
    const firstOpen = deferred<{
      version: 1;
      id: string;
      title: string;
      description: null;
      page: string;
      pages: string[];
      document_id: string;
      datasets: never[];
    }>();
    mockBoundary({
      report_list: () => listOf(PIPELINE, CHURN),
      report_open: ({ reportId }) =>
        reportId === "report-1"
          ? firstOpen.promise
          : {
              version: 1,
              id: "report-2",
              title: "Churn by cohort",
              description: null,
              page: "index.html",
              pages: ["index.html"],
              document_id: "newer",
              datasets: [],
            },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);

    await user.click(await screen.findByRole("button", { name: /^Open Q3 pipeline/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("report_open", { reportId: "report-1", page: null }));
    requestOpenReport("report-2");

    const viewer = await screen.findByRole("region", { name: "Report Churn by cohort" });
    expect(within(viewer).getByTitle("Churn by cohort")).toHaveAttribute("src", "http://xf-preview.localhost/newer");

    await act(() =>
      firstOpen.resolve({
        version: 1,
        id: "report-1",
        title: "Q3 pipeline",
        description: null,
        page: "index.html",
        pages: ["index.html"],
        document_id: "stale",
        datasets: [],
      }),
    );
    expect(within(viewer).getByTitle("Churn by cohort")).toHaveAttribute("src", "http://xf-preview.localhost/newer");
  });

  it("ignores an older list completion after a newer activation", async () => {
    const oldList = deferred<ReturnType<typeof listOf>>();
    let listCalls = 0;
    mockBoundary({
      report_list: () => {
        listCalls += 1;
        return listCalls === 1 ? oldList.promise : listOf(CHURN);
      },
    });
    const { rerender } = render(<ReportsView active />);
    await waitFor(() => expect(listCalls).toBe(1));

    rerender(<ReportsView active={false} />);
    rerender(<ReportsView active />);
    expect(await screen.findByText("Churn by cohort")).toBeInTheDocument();

    await act(() => oldList.resolve(listOf(PIPELINE)));
    expect(screen.getByText("Churn by cohort")).toBeInTheDocument();
    expect(screen.queryByText("Q3 pipeline")).not.toBeInTheDocument();
  });

  it("keeps two reports with the same name apart, by id", async () => {
    mockBoundary({
      report_list: () =>
        listOf(
          report({ id: "report-1", title: "Sales snapshot" }),
          report({ id: "report-2", title: "Sales snapshot", updated_at_ms: 1_783_400_000_000n }),
        ),
      report_delete: () => null,
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findAllByText("Sales snapshot");

    // Two rows reading the same thing is only a duplicate title. The CONTROLS
    // still resolve, because each names the time its own report last changed.
    const deletes = screen.getAllByRole("button", { name: /^Delete Sales snapshot/ });
    expect(deletes).toHaveLength(2);
    expect(new Set(deletes.map((button) => button.getAttribute("aria-label"))).size).toBe(2);

    await user.click(deletes[1]);
    await user.click(screen.getByRole("button", { name: "Delete" }));

    // An id crossed the boundary, not a name that matches both reports.
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("report_delete", { reportId: "report-2" }));
    expect(invokeMock.mock.calls.filter(([command]) => command === "report_delete")).toHaveLength(1);
  });

  it("says a name is already used, and renames to it anyway", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE, CHURN),
      report_rename: (args) => ({ ...CHURN, title: args.title as string }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Churn by cohort");

    await user.click(screen.getByRole("button", { name: /^Rename Churn by cohort/ }));
    const nameField = screen.getByRole("textbox", { name: /Name/ });
    await user.clear(nameField);
    await user.type(nameField, "  q3 PIPELINE  ");

    // Case-folded and trimmed: the reader sees one name whatever the bytes are.
    expect(await screen.findByText(/already called/)).toBeInTheDocument();

    // Advisory, never a refusal. Titles carry no uniqueness constraint and the
    // store could not enforce one — the title is inside the ciphertext.
    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeEnabled();
    await user.click(save);
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("report_rename", {
        reportId: "report-2",
        title: "q3 PIPELINE",
        description: null,
      }),
    );
  });

  it("drops a palette request when the read fails, so a later retry does not open it", async () => {
    let reads = 0;
    mockBoundary({
      report_list: () => {
        reads += 1;
        if (reads === 1) throw new Error("down");
        return listOf(PIPELINE, CHURN);
      },
    });
    const user = userEvent.setup();
    requestOpenReport("report-2");
    render(<ReportsView active />);

    expect(await screen.findByText("Reports unavailable")).toBeInTheDocument();
    expect(getPendingReportId()).toBeNull();
    await user.click(screen.getByRole("button", { name: "Try again" }));

    expect(await screen.findByText("Churn by cohort")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Report Churn by cohort" })).not.toBeInTheDocument();
  });

  it("opens the report the command palette named", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE, CHURN),
      report_open: () => ({
        version: 1,
        id: "report-2",
        title: "Churn by cohort",
        description: null,
        page: "index.html",
        pages: ["index.html"],
        document_id: "def456",
        datasets: [],
      }),
    });
    render(<ReportsView active />);
    await screen.findByText("Churn by cohort");

    requestOpenReport("report-2");

    const viewer = await screen.findByRole("region", { name: "Report Churn by cohort" });
    expect(within(viewer).getByTitle("Churn by cohort")).toBeInTheDocument();
  });
});

describe("report library reads", () => {
  beforeEach(() => resetReportsStore());

  it("still lands a list whose request the palette took over and then cancelled", async () => {
    const pending = deferred<unknown>();
    mockBoundary({ report_list: () => pending.promise });
    render(<ReportsView active />);

    // The palette's prefetch becomes the catalog's owner, then closes without
    // applying anything. The read the destination started must not be dropped.
    const prefetch = beginReportListRequest();
    cancelReportListRequest(prefetch);
    await act(async () => pending.resolve(listOf(PIPELINE)));

    expect(await screen.findByText("Q3 pipeline")).toBeInTheDocument();
  });

  it("lands a revisit's read when the palette opens and closes while it is in flight", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE) });
    const first = render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    first.unmount();

    const reread = deferred<unknown>();
    mockBoundary({ report_list: () => reread.promise });
    render(<ReportsView active />);

    // The palette opens (its prefetch takes the catalog) and closes without
    // choosing anything. The revisit's read must still replace the cached list.
    const prefetch = beginReportListRequest();
    cancelReportListRequest(prefetch);
    await act(async () => reread.resolve(listOf(CHURN)));

    expect(await screen.findByText("Churn by cohort")).toBeInTheDocument();
    expect(screen.queryByText("Q3 pipeline")).not.toBeInTheDocument();
  });

  it("shows no reports, and no count, after a failed read that follows a good one", async () => {
    let reads = 0;
    mockBoundary({
      report_list: () => {
        reads += 1;
        if (reads === 1) return listOf(PIPELINE);
        throw new Error("DA is unreachable");
      },
    });
    const user = userEvent.setup();
    const { rerender } = render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    rerender(<ReportsView active={false} />);
    rerender(<ReportsView active />);

    expect(await screen.findByText("Reports unavailable")).toBeInTheDocument();
    expect(getReportsSnapshot().reports).toEqual([]);
    expect(screen.queryByText("Q3 pipeline")).not.toBeInTheDocument();
    // A failed read is not a library of zero reports, so no count is claimed.
    const overview = within(screen.getByLabelText("Library overview"));
    expect(overview.getAllByText("—")).toHaveLength(2);
    expect(overview.queryByText("1")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(getReportsSnapshot().status).toBe("error"));
  });

  it("shows the skeleton on a first visit, until the library lands", async () => {
    const pending = deferred<unknown>();
    mockBoundary({ report_list: () => pending.promise });
    const { container } = render(<ReportsView active />);

    expect(container.querySelector(".skeleton-shimmer")).not.toBeNull();
    await act(async () => pending.resolve(listOf(PIPELINE)));
    expect(await screen.findByText("Q3 pipeline")).toBeInTheDocument();
    expect(container.querySelector(".skeleton-shimmer")).toBeNull();
  });

  it("keeps the cached library on a revisit, then replaces it when the read lands", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE) });
    const first = render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    first.unmount();

    const reread = deferred<unknown>();
    mockBoundary({ report_list: () => reread.promise });
    const { container } = render(<ReportsView active />);

    // The rows are drawn in the first render, with no skeleton over them.
    expect(screen.getByText("Q3 pipeline")).toBeInTheDocument();
    expect(container.querySelector(".skeleton-shimmer")).toBeNull();

    await act(async () => reread.resolve(listOf(CHURN)));
    expect(await screen.findByText("Churn by cohort")).toBeInTheDocument();
    expect(screen.queryByText("Q3 pipeline")).not.toBeInTheDocument();
    expect(invokeMock.mock.calls.filter(([command]) => command === "report_list")).toHaveLength(2);
  });

  it("shows the error state, not the cached rows, when the revisit read fails", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE) });
    const first = render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    first.unmount();

    const reread = deferred<unknown>();
    mockBoundary({ report_list: () => reread.promise });
    render(<ReportsView active />);
    expect(screen.getByText("Q3 pipeline")).toBeInTheDocument();

    await act(async () => reread.reject(new Error("DA is unreachable")));
    expect(await screen.findByText("Reports unavailable")).toBeInTheDocument();
    expect(screen.queryByText("Q3 pipeline")).not.toBeInTheDocument();
    expect(getReportsSnapshot().reports).toEqual([]);
  });

  it("shows the loading state again when the retry button is pressed", async () => {
    const retry = deferred<unknown>();
    let reads = 0;
    mockBoundary({
      report_list: () => {
        reads += 1;
        if (reads === 1) throw new Error("DA is unreachable");
        return retry.promise;
      },
    });
    const user = userEvent.setup();
    const { container } = render(<ReportsView active />);
    await screen.findByText("Reports unavailable");

    await user.click(screen.getByRole("button", { name: "Try again" }));

    expect(getReportsSnapshot().status).toBe("loading");
    expect(container.querySelector(".skeleton-shimmer")).not.toBeNull();
    await act(async () => retry.resolve(listOf(PIPELINE)));
    expect(await screen.findByText("Q3 pipeline")).toBeInTheDocument();
  });
});

describe("report workspace controls", () => {
  it("explains that the assistant saves reports, without creation or sync buttons", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE) });
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    expect(
      screen.getByText("Ask the assistant to save a dashboard in a chat to add a report here."),
    ).toBeInTheDocument();
    // The only controls beside the list are about reading it: a creation or sync
    // button under any name would fail here.
    const toolbar = screen.getByRole("button", { name: "List view" }).closest(".reports-toolbar");
    expect(toolbar).not.toBeNull();
    expect(within(toolbar as HTMLElement).queryByRole("button", { name: /new|create|sync|import|add/i })).toBeNull();
    expect(screen.getByPlaceholderText("Search library")).toBeInTheDocument();
  });
});

describe("report library browsing", () => {
  it("combines pinned filtering and search and clears both from the empty result", async () => {
    mockBoundary({ report_list: () => listOf({ ...PIPELINE, pinned: true }, CHURN) });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    await user.click(screen.getByRole("radio", { name: "Pinned" }));
    expect(drawnOrder()).toEqual(["Q3 pipeline"]);
    // A filter with no search is still a subset, and the summary says so.
    expect(screen.getByText("1 of 2 reports shown")).toBeInTheDocument();
    await user.type(screen.getByRole("searchbox", { name: "Search reports" }), "cohort");
    expect(screen.queryByRole("button", { name: /^Open / })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show all reports" }));
    expect(drawnOrder()).toEqual(["Q3 pipeline", "Churn by cohort"]);
  });

  it("sorts all results in list and grid without changing the stored catalog", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE, { ...CHURN, created_at_ms: PIPELINE.created_at_ms + 10n }) });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    const original = getReportsSnapshot().reports;
    await user.click(screen.getByRole("button", { name: "Sort reports" }));
    await user.click(screen.getByRole("menuitemradio", { name: "Name A–Z" }));
    expect(drawnOrder()).toEqual(["Churn by cohort", "Q3 pipeline"]);
    await user.click(screen.getByRole("button", { name: "List view" }));
    expect(screen.getByRole("table")).toBeInTheDocument();
    expect(drawnOrder()).toEqual(["Churn by cohort", "Q3 pipeline"]);
    await user.click(screen.getByRole("button", { name: "Sort reports" }));
    await user.click(screen.getByRole("menuitemradio", { name: "Newest created" }));
    expect(drawnOrder()).toEqual(["Churn by cohort", "Q3 pipeline"]);
    expect(getReportsSnapshot().reports).toBe(original);
    await user.click(screen.getByRole("button", { name: "Grid view" }));
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(drawnOrder()).toEqual(["Churn by cohort", "Q3 pipeline"]);
  });
});

describe("report ownership and sharing", () => {
  it("offers no Rename, Delete or Share on a report the viewer does not own", async () => {
    mockBoundary({ report_list: () => listOf(SHARED, PIPELINE) });
    render(<ReportsView active />);
    await screen.findByText("Shared pipeline");

    for (const verb of ["Rename", "Delete", "Share"]) {
      expect(screen.queryByRole("button", { name: new RegExp(`^${verb} Shared pipeline`) })).not.toBeInTheDocument();
    }
    // The reader's own controls stay.
    expect(screen.getByRole("button", { name: /^Pin Shared pipeline/ })).toBeEnabled();
    expect(screen.getByRole("button", { name: /^Hide Shared pipeline/ })).toBeEnabled();
    // The owner's report beside it keeps every control.
    expect(screen.getByRole("button", { name: /^Rename Q3 pipeline/ })).toBeEnabled();
  });

  it("enables Rename and Delete on a report the viewer owns", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE) });
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    expect(screen.getByRole("button", { name: /^Rename Q3 pipeline/ })).toBeEnabled();
    expect(screen.getByRole("button", { name: /^Delete Q3 pipeline/ })).toBeEnabled();
  });

  it("pins a report the viewer does not own", async () => {
    mockBoundary({
      report_list: () => listOf(SHARED),
      report_set_pinned: () => ({ ...SHARED, pinned: true }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);

    await user.click(await screen.findByRole("button", { name: /^Pin Shared pipeline/ }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("report_set_pinned", { reportId: "report-3", pinned: true }),
    );
    expect(await screen.findByRole("region", { name: "Pinned" })).toHaveTextContent("Shared pipeline");
  });

  it("names the owner of a shared report, in the card and in the table", async () => {
    mockBoundary({ report_list: () => listOf(SHARED) });
    const user = userEvent.setup();
    render(<ReportsView active />);

    await screen.findByText("Shared pipeline");
    expect(screen.getByText("Shared by Ada Lovelace")).toBeInTheDocument();

    // The table gives the owner and who can see it columns of their own.
    await user.click(screen.getByRole("button", { name: "List view" }));
    const row = screen.getByRole("button", { name: /^Open Shared pipeline/ }).closest("tr") as HTMLElement;
    const cells = within(row).getAllByRole("cell");
    const column = (name: string) =>
      cells[
        within(screen.getByRole("table"))
          .getAllByRole("columnheader")
          .findIndex((th) => th.textContent === name)
      ];
    expect(column("Owner")).toHaveTextContent("Ada Lovelace");
    expect(column("Visibility")).toHaveTextContent("Organization");
  });

  it("says a report is shared with the organization when the viewer owns it", async () => {
    mockBoundary({ report_list: () => listOf({ ...PIPELINE, scope: "global" }) });
    render(<ReportsView active />);

    expect(await screen.findByText("Shared with your organization")).toBeInTheDocument();
  });
});

describe("report viewer refresh and pages", () => {
  const document = (overrides: Record<string, unknown> = {}) => ({
    version: 1,
    id: "report-1",
    title: "Q3 pipeline",
    description: null,
    page: "index.html",
    pages: ["index.html"],
    document_id: "abc123",
    datasets: [],
    ...overrides,
  });

  const refreshed = {
    version: 1,
    report: REFRESHABLE,
    outcomes: [
      { version: 1, dataset_id: "sales", ok: true, rows: 3n, duration_ms: 10n, data_source: null, error: null },
    ],
  };

  it("takes over even when the page it replaces never finished loading", async () => {
    let opens = 0;
    mockBoundary({
      report_list: () => listOf(REFRESHABLE),
      report_open: () => document({ document_id: ++opens === 1 ? "abc123" : "fresh456" }),
      report_refresh: () => refreshed,
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");
    // The first page has not fired `load` when the reader refreshes.
    await user.click(screen.getByRole("button", { name: /^Refresh\b/ }));
    await waitFor(() => expect(opens).toBe(2));

    const viewer = screen.getByRole("region", { name: "Report Q3 pipeline" });
    const frames = await waitFor(() => {
      const found = within(viewer).getAllByTitle("Q3 pipeline");
      expect(found).toHaveLength(2);
      return found;
    });
    fireEvent.load(frames[1]);

    await waitFor(() => expect(within(viewer).getAllByTitle("Q3 pipeline")).toHaveLength(1));
    const only = within(viewer).getByTitle("Q3 pipeline");
    expect(only).toHaveAttribute("src", "http://xf-preview.localhost/fresh456");
    // Shown, not left behind a skeleton waiting for a load that already happened.
    expect(only).toHaveClass("opacity-100");
    expect(viewer.querySelector(".skeleton-shimmer")).toBeNull();
  });

  it("opens a failed refresh's details again even after the reader closed the last one's", async () => {
    mockBoundary({
      report_list: () => listOf(REFRESHABLE),
      report_open: () =>
        document({ datasets: [{ version: 1, dataset_id: "sales", executed_at_ms: null, row_count: null }] }),
      report_refresh: () => ({
        version: 1,
        report: REFRESHABLE,
        outcomes: [
          {
            version: 1,
            dataset_id: "sales",
            ok: false,
            rows: 0n,
            duration_ms: 0n,
            data_source: null,
            error: "Timeout",
          },
        ],
      }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: /^Refresh\b/ }));
    const details = await screen.findByRole("button", { name: "Details" });
    expect(details).toHaveAttribute("aria-expanded", "true");
    await user.click(details);
    expect(screen.getByRole("button", { name: "Details" })).toHaveAttribute("aria-expanded", "false");

    await user.click(screen.getByRole("button", { name: /^Refresh\b/ }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Details" })).toHaveAttribute("aria-expanded", "true"),
    );
    expect(screen.getByRole("alert")).toHaveTextContent("keeps its last stored rows");
  });

  it("brings the page in normally when a refresh follows a failed open", async () => {
    let opens = 0;
    mockBoundary({
      report_list: () => listOf(REFRESHABLE),
      report_open: () => {
        opens += 1;
        // The first open and its one retry fail; the open after the refresh works.
        if (opens <= 2) throw new Error("storage_unavailable");
        return document({ document_id: "fresh456" });
      },
      report_refresh: () => refreshed,
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");
    expect(await screen.findByText("This report could not be opened.")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /^Refresh\b/ }));

    const viewer = screen.getByRole("region", { name: "Report Q3 pipeline" });
    const frame = await within(viewer).findByTitle("Q3 pipeline");
    expect(frame).toHaveAttribute("src", "http://xf-preview.localhost/fresh456");
    expect(frame).not.toHaveClass("invisible");
    expect(screen.queryByText("This report could not be opened.")).not.toBeInTheDocument();
  });

  it("shows no Refresh control when DA does not allow one", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE), report_open: () => document() });
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    expect(screen.queryByRole("button", { name: /^Refresh\b/ })).not.toBeInTheDocument();
  });

  it("refreshes, re-opens the page, and confirms with a toast", async () => {
    const pending = deferred<unknown>();
    let opens = 0;
    mockBoundary({
      report_list: () => listOf(REFRESHABLE),
      report_open: () => {
        opens += 1;
        return document({ document_id: opens === 1 ? "abc123" : "fresh456" });
      },
      report_refresh: () => pending.promise,
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");
    // The page on screen has loaded: a refresh must keep it, not blank it.
    fireEvent.load(await screen.findByTitle("Q3 pipeline"));

    await user.click(screen.getByRole("button", { name: /^Refresh\b/ }));
    const busy = screen.getByRole("button", { name: /^Refresh\b/ });
    expect(busy).toBeDisabled();
    expect(busy).toHaveAttribute("aria-busy", "true");
    expect(screen.getByText("Refreshing report…")).toHaveAttribute("role", "status");
    // The strip says so too, and the button says what it is doing.
    expect(screen.getByText("Refreshing data…")).toBeInTheDocument();
    expect(within(busy).getByText("Refreshing")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("report_refresh", { reportId: "report-1" });

    await act(async () =>
      pending.resolve({
        version: 1,
        report: { ...REFRESHABLE, updated_at_ms: 1_783_900_000_000n },
        outcomes: [
          {
            version: 1,
            dataset_id: "sales",
            ok: true,
            rows: 12n,
            duration_ms: 1_250n,
            data_source: "warehouse",
            error: null,
          },
        ],
      }),
    );

    await waitFor(() => expect(opens).toBe(2));
    expect(getToasts().at(-1)?.text).toBe("Report refreshed");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByText("Refreshed 1 dataset · 12 rows")).toBeInTheDocument();
    const viewer = screen.getByRole("region", { name: "Report Q3 pipeline" });
    // The report stays on screen while the new page loads behind it: two frames,
    // the old one in view, the new one hidden, and no skeleton drawn over either.
    await waitFor(() => expect(within(viewer).getAllByTitle("Q3 pipeline")).toHaveLength(2));
    const [shown, behind] = within(viewer).getAllByTitle("Q3 pipeline");
    expect(shown).toHaveAttribute("src", "http://xf-preview.localhost/abc123");
    expect(shown).not.toHaveClass("invisible");
    expect(behind).toHaveAttribute("src", "http://xf-preview.localhost/fresh456");
    expect(behind).toHaveClass("invisible");
    expect(viewer.querySelector(".skeleton-shimmer")).toBeNull();

    // When the new page has loaded it takes over and the old frame is gone.
    fireEvent.load(behind);
    await waitFor(() => expect(within(viewer).getAllByTitle("Q3 pipeline")).toHaveLength(1));
    const only = within(viewer).getByTitle("Q3 pipeline");
    expect(only).toHaveAttribute("src", "http://xf-preview.localhost/fresh456");
    expect(only).not.toHaveClass("invisible");
  });

  it("moves focus into the viewer on open and back to the library on Back", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE), report_open: () => document() });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    expect(screen.getByRole("region", { name: "Report Q3 pipeline" })).toHaveFocus();

    await user.click(screen.getByRole("button", { name: "Back to reports" }));

    expect(await screen.findByRole("region", { name: "Saved reports" })).toHaveFocus();
  });

  it("records a refresh in the library even when the reader went back before it finished", async () => {
    const pending = deferred<unknown>();
    mockBoundary({
      report_list: () => listOf(REFRESHABLE),
      report_open: () => document(),
      report_refresh: () => pending.promise,
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");
    await user.click(screen.getByRole("button", { name: /^Refresh\b/ }));

    await user.click(screen.getByRole("button", { name: "Back to reports" }));
    await act(async () =>
      pending.resolve({
        version: 1,
        report: { ...REFRESHABLE, updated_at_ms: 1_783_900_000_000n },
        outcomes: [
          {
            version: 1,
            dataset_id: "sales",
            ok: true,
            rows: 12n,
            duration_ms: 1_250n,
            data_source: "warehouse",
            error: null,
          },
        ],
      }),
    );

    expect(getReportsSnapshot().reports.find((report) => report.id === REFRESHABLE.id)?.updated_at_ms).toBe(
      1_783_900_000_000n,
    );
    // The reader had already left: no success toast for a viewer that is gone.
    expect(getToasts().some((item) => item.text === "Report refreshed")).toBe(false);
  });

  it("shows the failed dataset in an alert and does not claim success", async () => {
    mockBoundary({
      report_list: () => listOf(REFRESHABLE),
      report_open: () => document(),
      report_refresh: () => ({
        version: 1,
        report: REFRESHABLE,
        outcomes: [
          {
            version: 1,
            dataset_id: "sales",
            ok: true,
            rows: 12n,
            duration_ms: 1_250n,
            data_source: "warehouse",
            error: null,
          },
          {
            version: 1,
            dataset_id: "churn",
            ok: false,
            rows: 0n,
            duration_ms: 0n,
            data_source: null,
            error: "Warehouse timeout",
          },
        ],
      }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: /^Refresh\b/ }));

    expect(await screen.findByText("1 of 2 datasets refreshed · 1 failed")).toBeInTheDocument();
    // A failure opens its own details, so the reader does not have to look for it.
    expect(screen.getByRole("alert")).toHaveTextContent("A dataset that failed keeps its last stored rows");
    expect(screen.getByText("Warehouse timeout")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Details" })).toHaveAttribute("aria-expanded", "true");
    expect(getToasts().some((item) => item.text === "Report refreshed")).toBe(false);
  });

  it("tells the reader when DA refuses the refresh", async () => {
    mockBoundary({
      report_list: () => listOf(REFRESHABLE),
      report_open: () => document(),
      report_refresh: () => {
        throw { version: 1, code: "forbidden" };
      },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: /^Refresh\b/ }));

    await waitFor(() => expect(getToasts().at(-1)?.text).toBe("You are not allowed to refresh this report"));
    expect(screen.getByRole("button", { name: /^Refresh\b/ })).toBeEnabled();
  });

  it("names the report in the Refresh control, so two open tabs of reports stay distinguishable", async () => {
    mockBoundary({ report_list: () => listOf(REFRESHABLE), report_open: () => document() });
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    expect(screen.getByRole("button", { name: /^Refresh Q3 pipeline — updated / })).toBeInTheDocument();
  });

  it("says a report with no stored queries cannot be refreshed", async () => {
    mockBoundary({
      report_list: () => listOf(REFRESHABLE),
      report_open: () => document(),
      report_refresh: () => {
        throw { version: 1, code: "not_refreshable" };
      },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: /^Refresh\b/ }));

    await waitFor(() =>
      expect(getToasts().at(-1)?.text).toBe("This report has no stored queries, so it cannot be refreshed"),
    );
  });

  it("does not claim a refresh when the page cannot be opened again", async () => {
    let opens = 0;
    mockBoundary({
      report_list: () => listOf(REFRESHABLE),
      report_open: () => {
        opens += 1;
        if (opens === 1) return document();
        throw { version: 1, code: "storage_unavailable" };
      },
      report_refresh: () => ({
        version: 1,
        report: REFRESHABLE,
        outcomes: [
          {
            version: 1,
            dataset_id: "sales",
            ok: true,
            rows: 12n,
            duration_ms: 1_250n,
            data_source: "warehouse",
            error: null,
          },
        ],
      }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: /^Refresh\b/ }));

    expect(await screen.findByText("This report could not be opened.")).toBeInTheDocument();
    expect(getToasts().some((item) => item.text === "Report refreshed")).toBe(false);
    expect(screen.getByRole("button", { name: /^Refresh\b/ })).toBeEnabled();
  });

  it("offers the pages of a multi-page report and opens the one chosen", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE),
      report_open: (args) =>
        document(
          args.page === "detail.html"
            ? { page: "detail.html", pages: ["index.html", "detail.html"], document_id: "detail789" }
            : { page: "index.html", pages: ["index.html", "detail.html"] },
        ),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    const index = await screen.findByRole("radio", { name: "index.html" });
    expect(index).toHaveAttribute("aria-checked", "true");
    await user.click(screen.getByRole("radio", { name: "detail.html" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("report_open", { reportId: "report-1", page: "detail.html" }),
    );
    const viewer = await screen.findByRole("region", { name: "Report Q3 pipeline" });
    await waitFor(() =>
      expect(within(viewer).getByTitle("Q3 pipeline")).toHaveAttribute("src", "http://xf-preview.localhost/detail789"),
    );
  });

  it("says a report is too large to open, and does not re-open it", async () => {
    let opens = 0;
    mockBoundary({
      report_list: () => listOf(PIPELINE),
      report_open: () => {
        opens += 1;
        throw { version: 1, code: "too_large_to_open" };
      },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user.click(await screen.findByText("Q3 pipeline"));

    expect(await screen.findByText("This report is too large to open.")).toBeInTheDocument();
    expect(opens).toBe(1);
  });
});

/** Opens the report whose title is given, from the list, and waits for the viewer. */
async function user_open(title: string) {
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: new RegExp(`^Open ${title} — `) }));
  await screen.findByRole("region", { name: `Report ${title}` });
}

describe("library visibility, sharing and kinds", () => {
  const HIDDEN = report({ id: "report-4", title: "Old forecast", hidden: true });
  const WORKBOOK = report({
    id: "report-5",
    title: "Quarterly workbook",
    kind: "workbook",
    page_count: 0,
    refreshable: true,
    can_refresh: true,
  });
  const patched = (base: ReportView, fields: Partial<ReportView>): ReportView => ({ ...base, ...fields });

  it("keeps a hidden report out of the list and the counts until the reader asks for it", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE, HIDDEN) });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    expect(screen.queryByText("Old forecast")).not.toBeInTheDocument();
    expect(screen.getByText("1 saved report")).toBeInTheDocument();

    await user.click(screen.getByRole("switch", { name: "Show hidden (1)" }));
    expect(await screen.findByText("Old forecast")).toBeInTheDocument();
    // The overview and the results line count the same reports.
    expect(screen.getByText("2 saved reports")).toBeInTheDocument();
    expect(within(screen.getByLabelText("Library overview")).getByText("2")).toBeInTheDocument();
    // The way back is on the row itself, and the row says it is hidden.
    expect(screen.getByText("Hidden")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Show Old forecast/ })).toBeInTheDocument();

    await user.click(screen.getByRole("switch", { name: "Showing hidden" }));
    expect(screen.queryByText("Old forecast")).not.toBeInTheDocument();
  });

  it("hides a report for this reader by intent, and says so", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE, CHURN),
      report_set_hidden: (args) => patched(PIPELINE, { hidden: args.hidden as boolean }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: /^Hide Q3 pipeline/ }));

    expect(invokeMock).toHaveBeenCalledWith("report_set_hidden", { reportId: "report-1", hidden: true });
    await waitFor(() => expect(screen.queryByText("Q3 pipeline")).not.toBeInTheDocument());
    expect(screen.getByRole("switch", { name: "Show hidden (1)" })).toBeInTheDocument();
    expect(getToasts().at(-1)?.text).toBe("Report hidden from your list");
  });

  it("keeps the report in the list when hiding it fails", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE),
      report_set_hidden: () => {
        throw new Error("storage_unavailable");
      },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    const before = getReportsSnapshot();

    await user.click(screen.getByRole("button", { name: /^Hide Q3 pipeline/ }));

    expect(await screen.findByText("This report could not be hidden. Try again.")).toBeInTheDocument();
    expect(screen.getByText("Q3 pipeline")).toBeInTheDocument();
    expect(getReportsSnapshot()).toEqual(before);
  });

  it("brings a hidden report back by intent", async () => {
    mockBoundary({
      report_list: () => listOf(HIDDEN),
      report_set_hidden: (args) => patched(HIDDEN, { hidden: args.hidden as boolean }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);

    // Everything is hidden: the library says so and offers the way back, instead
    // of reading as empty.
    expect(await screen.findByText("All reports are hidden")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show hidden reports" }));
    await user.click(await screen.findByRole("button", { name: /^Show Old forecast/ }));

    expect(invokeMock).toHaveBeenCalledWith("report_set_hidden", { reportId: "report-4", hidden: false });
    expect(await screen.findByText("1 saved report")).toBeInTheDocument();
  });

  it("filters to what the reader made, what others shared, and what they pinned", async () => {
    mockBoundary({ report_list: () => listOf({ ...PIPELINE, pinned: true }, CHURN, SHARED) });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    await user.click(screen.getByRole("radio", { name: "Mine" }));
    expect(drawnOrder()).toEqual(["Q3 pipeline", "Churn by cohort"]);
    await user.click(screen.getByRole("radio", { name: "Shared with me" }));
    expect(drawnOrder()).toEqual(["Shared pipeline"]);
    await user.click(screen.getByRole("radio", { name: "Pinned" }));
    expect(drawnOrder()).toEqual(["Q3 pipeline"]);
    await user.click(screen.getByRole("radio", { name: "All reports" }));
    expect(drawnOrder()).toHaveLength(3);
  });

  it("says what an empty filter means instead of a generic empty list", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE) });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    await user.click(screen.getByRole("radio", { name: "Shared with me" }));
    expect(await screen.findByText("No one has shared a report with you")).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "Pinned" }));
    expect(await screen.findByText("No pinned reports yet")).toBeInTheDocument();
  });

  it("asks before sharing, names who gets it, and sends only the scope", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE),
      report_set_scope: (args) => patched(PIPELINE, { scope: args.scope as ReportView["scope"] }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: /^Share Q3 pipeline/ }));
    const dialog = await screen.findByRole("alertdialog", { name: "Share with your organization?" });
    expect(dialog).toHaveTextContent("refresh it");
    expect(dialog).toHaveTextContent("does not create a public link");
    // Cancelling is the safe default and sends nothing.
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(invokeMock).not.toHaveBeenCalledWith("report_set_scope", expect.anything());

    await user.click(screen.getByRole("button", { name: /^Share Q3 pipeline/ }));
    await user.click(await screen.findByRole("button", { name: "Share with organization" }));

    expect(invokeMock).toHaveBeenCalledWith("report_set_scope", { reportId: "report-1", scope: "global" });
    expect(await screen.findByText("Shared with your organization")).toBeInTheDocument();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(getToasts().at(-1)?.text).toBe("Report shared with your organization");
  });

  it("makes a shared report private again, after asking", async () => {
    const shared = patched(PIPELINE, { scope: "global" });
    mockBoundary({
      report_list: () => listOf(shared),
      report_set_scope: (args) => patched(shared, { scope: args.scope as ReportView["scope"] }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: /^Share Q3 pipeline/ }));
    expect(await screen.findByRole("alertdialog", { name: "Make this report private?" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Make private" }));

    expect(invokeMock).toHaveBeenCalledWith("report_set_scope", { reportId: "report-1", scope: "user" });
    await waitFor(() => expect(screen.queryByText("Shared with your organization")).not.toBeInTheDocument());
  });

  it("offers sharing to the owner alone", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE, SHARED) });
    render(<ReportsView active />);
    await screen.findByText("Shared pipeline");

    expect(screen.getByRole("button", { name: /^Share Q3 pipeline/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Share Shared pipeline/ })).not.toBeInTheDocument();
  });

  it("explains a refusal from DA in the dialog, and a failure without changing the list", async () => {
    let calls = 0;
    mockBoundary({
      report_list: () => listOf(PIPELINE),
      report_set_scope: () => {
        calls += 1;
        throw calls === 1 ? { version: 1, code: "forbidden" } : new Error("storage_unavailable");
      },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    const before = getReportsSnapshot();

    await user.click(screen.getByRole("button", { name: /^Share Q3 pipeline/ }));
    await user.click(await screen.findByRole("button", { name: "Share with organization" }));
    expect(await screen.findByText("Only the owner can change who can open this report.")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Share with organization" }));
    expect(await screen.findByText("Sharing was not changed. Try again.")).toBeInTheDocument();
    expect(getReportsSnapshot()).toEqual(before);
  });

  it("lists a workbook by its kind, with no preview frame, and opens it as a file", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE, WORKBOOK),
      report_export: () => ({ version: 1, saved: true, file_name: "Quarterly workbook.xlsx" }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Quarterly workbook");

    expect(screen.getAllByText("Excel workbook").length).toBeGreaterThan(0);
    await user.click(screen.getByRole("button", { name: /^Open Quarterly workbook — / }));
    const viewer = await screen.findByRole("region", { name: "Report Quarterly workbook" });

    // A file has no page: nothing is opened, framed, or fetched for one.
    expect(invokeMock).not.toHaveBeenCalledWith("report_open", expect.objectContaining({ reportId: "report-5" }));
    expect(within(viewer).queryByTitle("Quarterly workbook")).not.toBeInTheDocument();
    expect(within(viewer).queryByRole("button", { name: /^Save to project/ })).not.toBeInTheDocument();

    await user.click(within(viewer).getByRole("button", { name: /^Download Quarterly workbook as Excel/ }));
    expect(invokeMock).toHaveBeenCalledWith("report_export", { reportId: "report-5", page: null });
    await waitFor(() => expect(getToasts().at(-1)?.text).toBe("Saved Quarterly workbook.xlsx"));
  });

  it("refreshes a workbook without trying to put a page back", async () => {
    mockBoundary({
      report_list: () => listOf(WORKBOOK),
      report_refresh: () => ({
        version: 1,
        report: WORKBOOK,
        outcomes: [
          { version: 1, dataset_id: "sales", ok: true, rows: 7n, duration_ms: 2_000n, data_source: null, error: null },
        ],
      }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user.click(await screen.findByRole("button", { name: /^Open Quarterly workbook — / }));
    await screen.findByRole("region", { name: "Report Quarterly workbook" });

    await user.click(screen.getByRole("button", { name: /^Refresh\b/ }));

    await waitFor(() => expect(getToasts().at(-1)?.text).toBe("Report refreshed"));
    expect(invokeMock).not.toHaveBeenCalledWith("report_open", expect.anything());
    expect(screen.getByText("Refreshed 1 dataset · 7 rows")).toBeInTheDocument();
  });

  it("does not claim a save when the reader dismissed the save dialog, and says when a write failed", async () => {
    let calls = 0;
    mockBoundary({
      report_list: () => listOf(WORKBOOK),
      report_export: () => {
        calls += 1;
        if (calls === 1) return { version: 1, saved: false, file_name: null };
        throw { version: 1, code: "export_failed" };
      },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user.click(await screen.findByRole("button", { name: /^Open Quarterly workbook — / }));
    const download = await screen.findByRole("button", { name: /^Download Quarterly workbook/ });

    await user.click(download);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("report_export", expect.anything()));
    expect(getToasts()).toHaveLength(0);

    await user.click(screen.getByRole("button", { name: /^Download Quarterly workbook/ }));
    await waitFor(() => expect(getToasts().at(-1)?.text).toBe("Could not export report"));
  });
});

describe("report viewer data age, export and project copy", () => {
  const document = (overrides: Record<string, unknown> = {}) => ({
    version: 1,
    id: "report-1",
    title: "Q3 pipeline",
    description: null,
    page: "index.html",
    pages: ["index.html"],
    document_id: "abc123",
    datasets: [],
    ...overrides,
  });

  it("states when each dataset was stored, and never that the data is current", async () => {
    mockBoundary({
      report_list: () => listOf(REFRESHABLE),
      report_open: () =>
        document({
          datasets: [
            { version: 1, dataset_id: "sales", executed_at_ms: 1_790_848_800_000n, row_count: 42n },
            { version: 1, dataset_id: "returns", executed_at_ms: null, row_count: null },
          ],
        }),
    });
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    const viewer = screen.getByRole("region", { name: "Report Q3 pipeline" });
    expect(await within(viewer).findByText(/Data stored/)).toBeInTheDocument();
    expect(within(viewer).getByText(/1 dataset has not been refreshed yet/)).toBeInTheDocument();
    // The per-dataset table is behind Details, closed until the reader asks.
    const details = within(viewer).getByRole("button", { name: "Details" });
    expect(details).toHaveAttribute("aria-expanded", "false");
    await userEvent.setup().click(details);
    expect(details).toHaveAttribute("aria-expanded", "true");
    const sales = within(viewer).getByRole("row", { name: /^sales/ });
    expect(within(sales).getByText("42")).toBeInTheDocument();
    const returns = within(viewer).getByRole("row", { name: /^returns/ });
    expect(within(returns).getByText("Not refreshed yet")).toBeInTheDocument();
    expect(within(viewer).queryByText(/current|up to date|live/i)).not.toBeInTheDocument();
  });

  it("says saved data was never refreshed, and says nothing for a static report", async () => {
    mockBoundary({ report_list: () => listOf(REFRESHABLE, CHURN), report_open: () => document() });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    expect(await screen.findByText(/Saved data has not been refreshed yet/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Back to reports" }));
    mockBoundary({
      report_list: () => listOf(REFRESHABLE, CHURN),
      report_open: () => document({ id: "report-2", title: "Churn by cohort" }),
    });
    await user_open("Churn by cohort");
    expect(screen.queryByText(/Saved data has not been refreshed yet/)).not.toBeInTheDocument();
  });

  it("lists what each dataset did on a refresh, with DA's reason for one that failed", async () => {
    mockBoundary({
      report_list: () => listOf(REFRESHABLE),
      report_open: () => document(),
      report_refresh: () => ({
        version: 1,
        report: REFRESHABLE,
        outcomes: [
          {
            version: 1,
            dataset_id: "sales",
            ok: true,
            rows: 12n,
            duration_ms: 2_000n,
            data_source: "warehouse",
            error: null,
          },
          {
            version: 1,
            dataset_id: "churn",
            ok: false,
            rows: 0n,
            duration_ms: 0n,
            data_source: null,
            error: "Warehouse timeout",
          },
        ],
      }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: /^Refresh\b/ }));

    expect(await screen.findByText("1 of 2 datasets refreshed · 1 failed")).toBeInTheDocument();
    const sales = screen.getByRole("row", { name: /^sales/ });
    expect(within(sales).getByText("12")).toBeInTheDocument();
    expect(within(sales).getByText("Refreshed in 2 s from warehouse")).toBeInTheDocument();
    const churn = screen.getByRole("row", { name: /^churn/ });
    expect(within(churn).getByText("Warehouse timeout")).toBeInTheDocument();
  });

  it("exports the page on screen to a file the reader names, passing no path and no markup", async () => {
    mockBoundary({
      report_list: () => listOf(PIPELINE),
      report_open: () => document({ pages: ["index.html", "detail.html"] }),
      report_export: () => ({ version: 1, saved: true, file_name: "Q3 pipeline.html" }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");
    const viewer = screen.getByRole("region", { name: "Report Q3 pipeline" });
    fireEvent.load(within(viewer).getByTitle("Q3 pipeline"));

    await user.click(await screen.findByRole("button", { name: "Export as an HTML file" }));

    expect(invokeMock).toHaveBeenCalledWith("report_export", { reportId: "report-1", page: "index.html" });
    await waitFor(() => expect(getToasts().at(-1)?.text).toBe("Saved Q3 pipeline.html"));
  });

  it("starts one export at a time", async () => {
    const pending = deferred<unknown>();
    let exports = 0;
    mockBoundary({
      report_list: () => listOf(PIPELINE),
      report_open: () => document(),
      report_export: () => {
        exports += 1;
        return pending.promise;
      },
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await user_open("Q3 pipeline");
    fireEvent.load(within(screen.getByRole("region", { name: "Report Q3 pipeline" })).getByTitle("Q3 pipeline"));

    const exportButton = await screen.findByRole("button", { name: "Export as an HTML file" });
    await user.click(exportButton);
    await user.click(exportButton);

    expect(exports).toBe(1);
    expect(screen.getByText("Exporting report…")).toHaveAttribute("role", "status");
    await act(async () => pending.resolve({ version: 1, saved: false, file_name: null }));
  });

  describe("Save to project", () => {
    const choices = (projects: { id: string; name: string }[], openFile = vi.fn()): ReportProjectChoices => ({
      projects,
      activeProjectId: "p2",
      openFile,
    });
    const renderWith = (value: ReportProjectChoices | null) =>
      render(
        <ReportProjectsContext.Provider value={value}>
          <ReportsView active />
        </ReportProjectsContext.Provider>,
      );
    const opened = async () => {
      await user_open("Q3 pipeline");
      fireEvent.load(within(screen.getByRole("region", { name: "Report Q3 pipeline" })).getByTitle("Q3 pipeline"));
    };
    const saved = (path: string) => ({
      version: 1,
      status: "saved",
      file: { version: 1, project_id: "p2", relative_path: path },
      limit: null,
      external_reference: false,
    });

    it("is offered only to a reader who has a project", async () => {
      mockBoundary({ report_list: () => listOf(PIPELINE), report_open: () => document() });
      renderWith(null);
      await user_open("Q3 pipeline");
      expect(screen.queryByRole("button", { name: /^Save to project/ })).not.toBeInTheDocument();

      cleanup();
      mockBoundary({ report_list: () => listOf(PIPELINE), report_open: () => document() });
      renderWith(choices([]));
      await user_open("Q3 pipeline");
      expect(screen.queryByRole("button", { name: /^Save to project/ })).not.toBeInTheDocument();
    });

    it("copies the page on screen into the project the reader is working in, and offers to open the file", async () => {
      const openFile = vi.fn();
      mockBoundary({
        report_list: () => listOf(PIPELINE),
        report_open: () => document(),
        report_save_library_to_project: () => saved("reports/Q3 pipeline.html"),
      });
      const user = userEvent.setup();
      renderWith(
        choices(
          [
            { id: "p1", name: "Sales" },
            { id: "p2", name: "Ops" },
          ],
          openFile,
        ),
      );
      await opened();

      await user.click(await screen.findByRole("button", { name: "Save to project: Q3 pipeline" }));
      const dialog = await screen.findByRole("dialog", { name: "Save to project" });
      // A snapshot, and it says so.
      expect(dialog).toHaveTextContent("does not refresh");
      await user.click(within(dialog).getByRole("button", { name: "Save to project" }));

      // Identity only: the page, the project, and the name. No bytes, no path.
      expect(invokeMock).toHaveBeenCalledWith("report_save_library_to_project", {
        reportId: "report-1",
        page: "index.html",
        projectId: "p2",
        title: "Q3 pipeline",
        allowDuplicate: false,
      });
      await waitFor(() => expect(getToasts().at(-1)?.text).toBe("Saved to Ops as reports/Q3 pipeline.html"));
      getToasts().at(-1)?.action?.onSelect();
      expect(openFile).toHaveBeenCalledWith("p2", "reports/Q3 pipeline.html");
    });

    it("treats a repeat as a choice, and a copy on purpose as a new file", async () => {
      let calls = 0;
      mockBoundary({
        report_list: () => listOf(PIPELINE),
        report_open: () => document(),
        report_save_library_to_project: () => {
          calls += 1;
          return calls === 1
            ? {
                version: 1,
                status: "duplicate",
                file: { version: 1, project_id: "p1", relative_path: "reports/Q3 pipeline.html" },
                limit: null,
                external_reference: false,
              }
            : saved("reports/Q3 pipeline (2).html");
        },
      });
      const user = userEvent.setup();
      renderWith({ ...choices([{ id: "p1", name: "Sales" }]), activeProjectId: "p1" });
      await opened();

      await user.click(await screen.findByRole("button", { name: "Save to project: Q3 pipeline" }));
      await user.click(
        within(await screen.findByRole("dialog", { name: "Save to project" })).getByRole("button", {
          name: "Save to project",
        }),
      );
      expect(await screen.findByText(/Already saved as “reports\/Q3 pipeline.html”/)).toBeInTheDocument();

      await user.click(screen.getByRole("button", { name: "Save a copy" }));
      expect(invokeMock).toHaveBeenLastCalledWith(
        "report_save_library_to_project",
        expect.objectContaining({ allowDuplicate: true }),
      );
      await waitFor(() => expect(getToasts().at(-1)?.text).toBe("Saved to Sales as reports/Q3 pipeline (2).html"));
    });

    it("says why when the project folder cannot be reached, and keeps the dialog open", async () => {
      mockBoundary({
        report_list: () => listOf(PIPELINE),
        report_open: () => document(),
        report_save_library_to_project: () => {
          throw { version: 1, code: "project_unavailable" };
        },
      });
      const user = userEvent.setup();
      renderWith({ ...choices([{ id: "p1", name: "Sales" }]), activeProjectId: "p1" });
      await opened();

      await user.click(await screen.findByRole("button", { name: "Save to project: Q3 pipeline" }));
      await user.click(
        within(await screen.findByRole("dialog", { name: "Save to project" })).getByRole("button", {
          name: "Save to project",
        }),
      );

      expect(await screen.findByText(/project folder could not be reached/)).toBeInTheDocument();
      expect(screen.getByRole("dialog", { name: "Save to project" })).toBeInTheDocument();
    });
  });
});

describe("filtering by project", () => {
  const SALES_A = report({ id: "report-1", title: "Q3 pipeline" });
  const SALES_B = report({ id: "report-2", title: "Churn by cohort", updated_at_ms: 1_783_400_000_000n });
  const OPS = report({ id: "report-3", title: "Ops board", updated_at_ms: 1_783_300_000_000n });
  const REGULAR = report({ id: "report-4", title: "Regular chat report", updated_at_ms: 1_783_200_000_000n });
  const ORIGINS = [
    originOfReport("report-1", "p-sales", "Sales"),
    originOfReport("report-2", "p-sales", "Sales"),
    originOfReport("report-3", "p-ops", "Ops"),
  ];
  const withProjects = () => ({ ...listOf(SALES_A, SALES_B, OPS, REGULAR), project_origins: ORIGINS });
  const patched = (base: ReportView, fields: Partial<ReportView>): ReportView => ({ ...base, ...fields });
  const choose = async (user: ReturnType<typeof userEvent.setup>, name: string) => {
    await user.click(screen.getByRole("button", { name: "Project" }));
    await user.click(await screen.findByRole("menuitemradio", { name }));
  };

  it("offers no project control while no report is linked to a project", async () => {
    mockBoundary({ report_list: () => listOf(PIPELINE, CHURN) });
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    expect(screen.queryByRole("button", { name: "Project" })).not.toBeInTheDocument();
  });

  it("lists each project with its count, then shows only that project's reports", async () => {
    mockBoundary({ report_list: withProjects });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: "Project" }));
    const labels = (await screen.findAllByRole("menuitemradio")).map((item) => item.textContent);
    expect(labels).toEqual(["All projects", "No project", "Ops (1)", "Sales (2)"]);
    await user.click(screen.getByRole("menuitemradio", { name: "Sales (2)" }));

    expect(drawnOrder()).toEqual(["Q3 pipeline", "Churn by cohort"]);
    expect(screen.getByText("2 of 4 reports shown")).toBeInTheDocument();
  });

  it("shows reports from regular chats under No project, and everything under All projects", async () => {
    mockBoundary({ report_list: withProjects });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    await choose(user, "No project");
    expect(drawnOrder()).toEqual(["Regular chat report"]);

    await choose(user, "All projects");
    expect(drawnOrder()).toHaveLength(4);
  });

  it("names the project on a linked report and on no other", async () => {
    mockBoundary({ report_list: withProjects });
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    const card = (title: string) => openButtonFor(title).closest(".report-card") as HTMLElement;
    const chipOf = (title: string) => card(title).querySelector(".report-project-chip");
    expect(chipOf("Q3 pipeline")).toHaveTextContent("Sales");
    expect(chipOf("Ops board")).toHaveTextContent("Ops");
    expect(chipOf("Regular chat report")).toBeNull();
  });

  it("combines with the other filters, and its empty result names the project and clears everything", async () => {
    mockBoundary({
      report_list: () => ({
        ...listOf(SALES_A, SHARED),
        project_origins: [originOfReport("report-1", "p-sales", "Sales")],
      }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    await choose(user, "Sales (1)");
    await user.click(screen.getByRole("radio", { name: "Shared with me" }));

    expect(await screen.findByText("No reports in Sales")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show all reports" }));
    expect(drawnOrder()).toEqual(["Q3 pipeline", "Shared pipeline"]);
    expect(screen.getByRole("radio", { name: "All reports" })).toBeChecked();
  });

  it("keeps a report's project through a pin, which returns a new row", async () => {
    mockBoundary({
      report_list: () => ({ ...listOf(SALES_A), project_origins: [originOfReport("report-1", "p-sales", "Sales")] }),
      report_set_pinned: (args) => patched(SALES_A, { pinned: args.pinned as boolean }),
    });
    const user = userEvent.setup();
    render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");

    await user.click(screen.getByRole("button", { name: /^Pin Q3 pipeline/ }));
    await screen.findByRole("button", { name: /^Unpin Q3 pipeline/ });

    expect(document.querySelector(".report-project-chip")).toHaveTextContent("Sales");
    expect(screen.getByRole("button", { name: "Project" })).toBeInTheDocument();
  });

  it("names the project in the report's own header", async () => {
    mockBoundary({
      report_list: () => ({ ...listOf(SALES_A), project_origins: [originOfReport("report-1", "p-sales", "Sales")] }),
      report_open: () => ({
        version: 1,
        id: "report-1",
        title: "Q3 pipeline",
        description: null,
        page: "index.html",
        pages: ["index.html"],
        document_id: "abc123",
        datasets: [],
      }),
    });
    render(<ReportsView active />);
    await user_open("Q3 pipeline");

    const viewer = screen.getByRole("region", { name: "Report Q3 pipeline" });
    expect(viewer.querySelector(".report-project-chip")).toHaveTextContent("Sales");
  });

  it("falls back to every report when the chosen project is no longer anyone's", async () => {
    let origins = ORIGINS;
    mockBoundary({ report_list: () => ({ ...listOf(SALES_A, SALES_B, OPS, REGULAR), project_origins: origins }) });
    const user = userEvent.setup();
    const { rerender } = render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    await choose(user, "Sales (2)");
    expect(drawnOrder()).toEqual(["Q3 pipeline", "Churn by cohort"]);

    // Sales was removed on this device: its links are gone on the next read.
    origins = ORIGINS.filter((origin) => origin.project_id !== "p-sales");
    rerender(<ReportsView active={false} />);
    rerender(<ReportsView active />);

    await waitFor(() => expect(drawnOrder()).toHaveLength(4));
    expect(screen.getByRole("button", { name: "Project" })).toHaveTextContent("All projects");
  });

  it("does not bring a dropped project's filter back when a project of that id returns", async () => {
    let origins = ORIGINS;
    mockBoundary({ report_list: () => ({ ...listOf(SALES_A, SALES_B, OPS, REGULAR), project_origins: origins }) });
    const user = userEvent.setup();
    const { rerender } = render(<ReportsView active />);
    await screen.findByText("Q3 pipeline");
    await choose(user, "Sales (2)");

    origins = ORIGINS.filter((origin) => origin.project_id !== "p-sales");
    rerender(<ReportsView active={false} />);
    rerender(<ReportsView active />);
    await waitFor(() => expect(drawnOrder()).toHaveLength(4));

    origins = ORIGINS;
    rerender(<ReportsView active={false} />);
    rerender(<ReportsView active />);
    await waitFor(() => expect(screen.getByRole("button", { name: "Project" })).toBeInTheDocument());
    expect(drawnOrder()).toHaveLength(4);
    expect(screen.getByRole("button", { name: "Project" })).toHaveTextContent("All projects");
  });
});
