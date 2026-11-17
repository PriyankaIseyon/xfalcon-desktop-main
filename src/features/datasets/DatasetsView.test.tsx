import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DatasetListResult, DatasetView, UploadQueueV1 } from "../../lib/generated/contracts";
import { DatasetsView } from "./DatasetsView";
import { enqueueDroppedDataset, resetDatasetsStore } from "./datasetsStore";
import { applyUploadQueue, resetUploadQueueForTests } from "../uploads/uploadQueueStore";
import { getToasts, resetToastsForTests } from "../../ui/toastStore";

const invokeMock = vi.hoisted(() => vi.fn());
const listenMock = vi.hoisted(() => vi.fn());
let queueRevision = 0;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  convertFileSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

function dataset(overrides: Partial<DatasetView> & { id: string; display_name: string }): DatasetView {
  return {
    version: 1,
    status: "ready",
    created_at_ms: 1_783_500_000_000n,
    updated_at_ms: 1_783_500_000_000n,
    ...overrides,
  };
}

const SALES = dataset({ id: "d-1", display_name: "Sales Q3", created_at_ms: 300n });
const INVENTORY = dataset({ id: "d-2", display_name: "Inventory", created_at_ms: 100n });

function listOf(datasets: DatasetView[], overrides: Partial<DatasetListResult> = {}): DatasetListResult {
  return { version: 1, datasets, uploads_enabled: true, truncated: false, ...overrides };
}

function mockBoundary(handlers: Record<string, (args: Record<string, unknown>) => unknown>) {
  invokeMock.mockImplementation(async (command: string, args: Record<string, unknown>) => {
    const handler = handlers[command];
    if (!handler) throw new Error(`Unexpected invoke: ${command}`);
    return handler(args ?? {});
  });
}

function queueOf(uploads: UploadQueueV1["uploads"]): UploadQueueV1 {
  return { version: 1, revision: ++queueRevision, uploads };
}

afterEach(cleanup);

beforeEach(() => {
  invokeMock.mockReset();
  listenMock.mockReset();
  listenMock.mockResolvedValue(() => {});
  resetDatasetsStore();
  resetUploadQueueForTests();
  resetToastsForTests();
  queueRevision = 0;
});

describe("the datasets destination", () => {
  it("lists what the workspace holds", async () => {
    mockBoundary({ dataset_list: () => listOf([SALES, INVENTORY]) });
    render(<DatasetsView />);

    await waitFor(() => expect(screen.getByText("Sales Q3")).toBeInTheDocument());
    expect(screen.getByText("Inventory")).toBeInTheDocument();
    expect(screen.getByText("2 datasets")).toBeInTheDocument();
    const overview = screen.getByLabelText("Dataset overview");
    expect(within(overview).getByText("2")).toBeInTheDocument();
    expect(within(overview).getByText("0")).toBeInTheDocument();
  });

  it("asks the workspace for the catalog once, when it opens", async () => {
    // The shell mounts this only while Datasets is the destination, so mount is
    // open — an account that never comes here never asks for a catalog. That
    // guarantee is asserted where it lives, in the shell's own tests.
    mockBoundary({ dataset_list: () => listOf([SALES]) });
    render(<DatasetsView />);

    await waitFor(() => expect(screen.getByText("Sales Q3")).toBeInTheDocument());
    expect(invokeMock.mock.calls.filter(([command]) => command === "dataset_list")).toHaveLength(1);
  });

  it("shows the skeleton on a first visit, until the list lands", async () => {
    let finishList!: (value: DatasetListResult) => void;
    mockBoundary({
      dataset_list: () =>
        new Promise<DatasetListResult>((resolve) => {
          finishList = resolve;
        }),
    });
    const { container } = render(<DatasetsView />);

    expect(container.querySelector(".skeleton-shimmer")).not.toBeNull();
    act(() => {
      finishList(listOf([SALES]));
    });
    await screen.findByText("Sales Q3");
    expect(container.querySelector(".skeleton-shimmer")).toBeNull();
  });

  it("keeps the cached list on a revisit, then replaces it when the read lands", async () => {
    mockBoundary({ dataset_list: () => listOf([SALES]) });
    const first = render(<DatasetsView />);
    await screen.findByText("Sales Q3");
    first.unmount();

    let finishReread!: (value: DatasetListResult) => void;
    mockBoundary({
      dataset_list: () =>
        new Promise<DatasetListResult>((resolve) => {
          finishReread = resolve;
        }),
    });
    const { container } = render(<DatasetsView />);

    // The rows are drawn in the first render, with no skeleton over them.
    expect(screen.getByText("Sales Q3")).toBeInTheDocument();
    expect(container.querySelector(".skeleton-shimmer")).toBeNull();

    finishReread(listOf([INVENTORY]));
    await waitFor(() => expect(screen.queryByText("Sales Q3")).not.toBeInTheDocument());
    expect(screen.getByText("Inventory")).toBeInTheDocument();
    expect(invokeMock.mock.calls.filter(([command]) => command === "dataset_list")).toHaveLength(2);
  });

  it("shows the load error when the revisit read fails, as a first visit does", async () => {
    mockBoundary({ dataset_list: () => listOf([SALES]) });
    const first = render(<DatasetsView />);
    await screen.findByText("Sales Q3");
    first.unmount();

    mockBoundary({
      dataset_list: () => {
        throw { version: 1, code: "network", step_index: null };
      },
    });
    render(<DatasetsView />);

    expect(await screen.findByText("Datasets could not be loaded")).toBeInTheDocument();
    expect(getToasts().some((item) => item.text === "The dataset list could not be refreshed.")).toBe(false);
  });

  it("offers the empty state when there is nothing yet", async () => {
    mockBoundary({ dataset_list: () => listOf([]) });
    render(<DatasetsView />);

    await waitFor(() => expect(screen.getByText("No datasets yet")).toBeInTheDocument());
    expect(screen.getAllByRole("button", { name: /Upload dataset/ }).length).toBeGreaterThan(0);
  });

  it("opens only one native picker across both upload controls", async () => {
    let finishPick!: (value: unknown) => void;
    const picking = new Promise((resolve) => {
      finishPick = resolve;
    });
    mockBoundary({
      dataset_list: () => listOf([]),
      dataset_pick: () => picking,
      dataset_preview: () => {
        throw { version: 1, code: "unreadable", step_index: null };
      },
    });
    render(<DatasetsView />);
    await screen.findByText("No datasets yet");
    const buttons = screen.getAllByRole("button", { name: "Upload dataset" });

    await userEvent.click(buttons[0]);
    buttons[1].click();

    expect(invokeMock.mock.calls.filter(([command]) => command === "dataset_pick")).toHaveLength(1);
    finishPick({
      version: 1,
      id: "staged-picker",
      display_name: "sales.csv",
      size: 10,
      format: "csv",
      worksheets: [],
    });
    await screen.findByText("sales.csv");
  });

  it("says why uploads are unavailable and still lists what is there", async () => {
    mockBoundary({ dataset_list: () => listOf([SALES], { uploads_enabled: false }) });
    render(<DatasetsView />);

    await waitFor(() => expect(screen.getByText("Uploads are turned off")).toBeInTheDocument());
    expect(screen.getByText("Sales Q3")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Upload dataset/ })).toBeDisabled();
  });

  it("says when the list is only part of the workspace's", async () => {
    mockBoundary({ dataset_list: () => listOf([SALES], { truncated: true }) });
    render(<DatasetsView />);
    await waitFor(() => expect(screen.getByText("Showing part of the list")).toBeInTheDocument());
  });

  it("reports a failed load with a way back", async () => {
    let attempts = 0;
    mockBoundary({
      dataset_list: () => {
        attempts += 1;
        if (attempts === 1) throw { version: 1, code: "network", step_index: null };
        return listOf([SALES]);
      },
    });
    render(<DatasetsView />);

    await waitFor(() => expect(screen.getByText("Datasets could not be loaded")).toBeInTheDocument());
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(screen.getByText("Sales Q3")).toBeInTheDocument());
  });

  it("maps an unknown boundary code to safe refusal copy", async () => {
    mockBoundary({
      dataset_list: () => {
        throw { version: 1, code: "future_internal_code", step_index: 99 };
      },
    });
    render(<DatasetsView />);

    expect(await screen.findByText("The workspace refused that. You can try again.")).toBeInTheDocument();
    expect(screen.queryByText(/future_internal_code/)).not.toBeInTheDocument();
  });

  it("filters by name", async () => {
    mockBoundary({ dataset_list: () => listOf([SALES, INVENTORY]) });
    render(<DatasetsView />);
    await waitFor(() => expect(screen.getByText("Sales Q3")).toBeInTheDocument());

    await userEvent.type(screen.getByRole("searchbox", { name: "Search datasets" }), "invent");
    await waitFor(() => expect(screen.queryByText("Sales Q3")).not.toBeInTheDocument());
    expect(screen.getByText("Inventory")).toBeInTheDocument();
    expect(screen.getByText("1 of 2 datasets")).toBeInTheDocument();
  });

  it("offers the way back when a search matches nothing", async () => {
    mockBoundary({ dataset_list: () => listOf([SALES, INVENTORY]) });
    render(<DatasetsView />);
    await waitFor(() => expect(screen.getByText("Sales Q3")).toBeInTheDocument());

    await userEvent.type(screen.getByRole("searchbox", { name: "Search datasets" }), "nothing");
    await waitFor(() => expect(screen.queryByText("Sales Q3")).not.toBeInTheDocument());
    // Not "Clear search": the search field already has a control by that name.
    await userEvent.click(screen.getByRole("button", { name: "Show all datasets" }));
    expect(await screen.findByText("Sales Q3")).toBeInTheDocument();
  });

  it("does not claim a partial catalog covers the whole workspace", async () => {
    mockBoundary({ dataset_list: () => listOf([SALES], { truncated: true }) });
    render(<DatasetsView />);
    await screen.findByText("Sales Q3");

    await userEvent.type(screen.getByRole("searchbox", { name: "Search datasets" }), "nothing");

    expect(
      await screen.findByText("No dataset in this partial list matches. Try another part of the name."),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Every dataset in this workspace/)).not.toBeInTheDocument();
  });

  it("sorts by the column that was clicked, and reverses on a second click", async () => {
    mockBoundary({ dataset_list: () => listOf([SALES, INVENTORY]) });
    render(<DatasetsView />);
    await waitFor(() => expect(screen.getByText("Sales Q3")).toBeInTheDocument());

    const names = () =>
      screen
        .getAllByRole("row")
        .slice(1)
        .map((row) => row.querySelector("td")?.textContent);

    // Newest first by default.
    expect(names()).toEqual(["Sales Q3", "Inventory"]);

    await userEvent.click(screen.getByRole("button", { name: "Name" }));
    expect(names()).toEqual(["Inventory", "Sales Q3"]);
    expect(screen.getByRole("columnheader", { name: /Name/ })).toHaveAttribute("aria-sort", "ascending");

    await userEvent.click(screen.getByRole("button", { name: "Name" }));
    expect(names()).toEqual(["Sales Q3", "Inventory"]);
    expect(screen.getByRole("columnheader", { name: /Name/ })).toHaveAttribute("aria-sort", "descending");
  });

  it("requires the name to be typed before it will delete", async () => {
    mockBoundary({
      dataset_list: () => listOf([SALES]),
      dataset_delete: () => null,
    });
    render(<DatasetsView />);
    await waitFor(() => expect(screen.getByText("Sales Q3")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: "Delete Sales Q3" }));

    expect(invokeMock).not.toHaveBeenCalledWith("dataset_delete", expect.anything());
    await userEvent.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Keep dataset" }));
    expect(screen.getByText("Sales Q3")).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("dataset_delete", expect.anything());
    await userEvent.click(screen.getByRole("button", { name: "Delete Sales Q3" }));

    const dialog = screen.getByRole("alertdialog");
    const confirm = within(dialog).getByRole("button", { name: "Delete dataset" });
    expect(confirm).toBeDisabled();

    await userEvent.type(within(dialog).getByRole("textbox"), "Sales Q");
    expect(confirm).toBeDisabled();

    await userEvent.type(within(dialog).getByRole("textbox"), "3");
    expect(confirm).toBeEnabled();
    await userEvent.click(confirm);

    await waitFor(() => expect(screen.queryByText("Sales Q3")).not.toBeInTheDocument());
    expect(invokeMock).toHaveBeenCalledWith("dataset_delete", { datasetId: "d-1" });
  });

  it("puts a row back when the delete fails", async () => {
    mockBoundary({
      dataset_list: () => listOf([SALES]),
      dataset_delete: () => {
        throw { version: 1, code: "server_refused", step_index: null };
      },
    });
    render(<DatasetsView />);
    await waitFor(() => expect(screen.getByText("Sales Q3")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: "Delete Sales Q3" }));
    const dialog = screen.getByRole("alertdialog");
    await userEvent.type(within(dialog).getByRole("textbox"), "Sales Q3");
    await userEvent.click(within(dialog).getByRole("button", { name: "Delete dataset" }));

    // The list must never show a change that did not happen. Asserted on the
    // table rather than by text, because the dialog names the dataset too — and
    // the dialog stays open on purpose, so the person can try again.
    await waitFor(() => expect(screen.getAllByRole("row")).toHaveLength(2));
    const retained = screen.getByRole("alertdialog");
    expect(within(retained).getByRole("textbox")).toHaveValue("Sales Q3");
    expect(within(retained).getByText("The workspace refused that. You can try again.")).toBeInTheDocument();
  });

  it("does not let an older catalog response restore a confirmed deletion", async () => {
    let listCalls = 0;
    let finishOlderList!: (value: DatasetListResult) => void;
    const olderList = new Promise<DatasetListResult>((resolve) => {
      finishOlderList = resolve;
    });
    mockBoundary({
      dataset_list: () => {
        listCalls += 1;
        return listCalls === 1 ? listOf([SALES]) : olderList;
      },
      dataset_delete: () => null,
    });
    render(<DatasetsView />);
    await screen.findByText("Sales Q3");

    applyUploadQueue(
      queueOf([
        {
          version: 1,
          id: "u-ready",
          purpose: "persistent_dataset",
          state: "ready",
          bytes_sent: 10,
          size: 10,
          display_name: "new.csv",
          safe_error: null,
        },
      ]),
    );
    await waitFor(() => expect(listCalls).toBe(2));

    await userEvent.click(screen.getByRole("button", { name: "Delete Sales Q3" }));
    const dialog = screen.getByRole("alertdialog");
    await userEvent.type(within(dialog).getByRole("textbox"), "Sales Q3");
    await userEvent.click(within(dialog).getByRole("button", { name: "Delete dataset" }));
    await waitFor(() => expect(screen.queryByText("Sales Q3")).not.toBeInTheDocument());

    finishOlderList(listOf([SALES]));
    await Promise.resolve();
    expect(screen.queryByText("Sales Q3")).not.toBeInTheDocument();
  });

  it("confirms dropped files one at a time, and does not lose the rest of the drop", async () => {
    // One gesture can drop eight files, and each one is already staged in the
    // backend. A dialog that only showed the last would leave the others held
    // with nothing able to confirm or discard them.
    mockBoundary({
      dataset_list: () => listOf([]),
      dataset_preview: () => {
        throw { version: 1, code: "unreadable", step_index: null };
      },
      dataset_stage_discard: () => null,
    });
    const staged = (id: string, name: string) => ({
      version: 1 as const,
      id,
      display_name: name,
      size: 2048,
      format: "csv" as const,
      worksheets: [],
    });

    render(<DatasetsView />);
    await screen.findByText("No datasets yet");

    enqueueDroppedDataset(staged("s-1", "january.csv"));
    enqueueDroppedDataset(staged("s-2", "february.csv"));

    const user = userEvent.setup();
    // The dialog names the file on its card, not in its title — the title
    // says which step this is.
    await screen.findByText("january.csv");
    await user.click(screen.getByRole("button", { name: "Close" }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("dataset_stage_discard", { stagedId: "s-1" });
    });
    // The second file's turn comes when the first one's is over.
    await screen.findByText("february.csv");
  });

  it("shows dataset transfers and leaves composer attachments to the composer", async () => {
    mockBoundary({ dataset_list: () => listOf([]) });
    render(<DatasetsView />);
    await waitFor(() => expect(screen.getByText("No datasets yet")).toBeInTheDocument());

    applyUploadQueue(
      queueOf([
        {
          version: 1,
          id: "u-1",
          purpose: "persistent_dataset",
          state: "uploading",
          bytes_sent: 500,
          size: 1000,
          display_name: "sales.csv",
          safe_error: null,
        },
        {
          version: 1,
          id: "u-2",
          purpose: "chat_attachment",
          state: "uploading",
          bytes_sent: 10,
          size: 100,
          display_name: "chart.png",
          safe_error: null,
          conversation_id: "conv-1",
        },
      ]),
    );

    await waitFor(() => expect(screen.getByText("sales.csv")).toBeInTheDocument());
    expect(screen.queryByText("chart.png")).not.toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: /sales.csv/ })).toHaveAttribute("aria-valuenow", "50");
  });

  it("clears a finished transfer and puts its dataset in the table", async () => {
    // The workspace only holds the dataset once the transfer settles, so the
    // row cannot appear when the upload starts. Nothing re-read the catalog at
    // that moment either, which left a completed transfer reading "Uploaded"
    // above a table that did not contain it.
    let listed: DatasetView[] = [];
    mockBoundary({ dataset_list: () => listOf(listed) });
    render(<DatasetsView />);
    await waitFor(() => expect(screen.getByText("No datasets yet")).toBeInTheDocument());

    const transfer = (state: "uploading" | "ready") => ({
      version: 1 as const,
      id: "u-1",
      purpose: "persistent_dataset" as const,
      state,
      bytes_sent: state === "ready" ? 1000 : 500,
      size: 1000,
      display_name: "sales.csv",
      safe_error: null,
    });

    applyUploadQueue(queueOf([transfer("uploading")]));
    await waitFor(() => expect(screen.getByRole("progressbar", { name: /sales.csv/ })).toBeInTheDocument());

    listed = [dataset({ id: "d-9", display_name: "sales.csv" })];
    applyUploadQueue(queueOf([transfer("ready")]));

    // The progress row goes, and the dataset arrives in its place.
    await waitFor(() => expect(screen.queryByRole("progressbar")).not.toBeInTheDocument());
    await waitFor(() => expect(screen.getByRole("cell", { name: "sales.csv" })).toBeInTheDocument());
  });

  it("keeps a failed transfer on screen, because it holds the reason and the retry", async () => {
    mockBoundary({ dataset_list: () => listOf([]) });
    render(<DatasetsView />);
    await waitFor(() => expect(screen.getByText("No datasets yet")).toBeInTheDocument());

    applyUploadQueue(
      queueOf([
        {
          version: 1,
          id: "u-1",
          purpose: "persistent_dataset",
          state: "failed",
          bytes_sent: 0,
          size: 1000,
          display_name: "sales.csv",
          safe_error: "The workspace refused the file.",
        },
      ]),
    );

    await waitFor(() => expect(screen.getByText("The workspace refused the file.")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("saves a dataset's only file from its row, without opening the drawer", async () => {
    mockBoundary({
      dataset_list: () => listOf([SALES]),
      dataset_detail: () => ({
        version: 1,
        dataset: SALES,
        uploads_enabled: true,
        files: [
          {
            file_id: "f-1",
            table_name: "sales_q3",
            row_count: 10n,
            column_count: 3,
            size_bytes: 2048n,
            original_filename: "sales.csv",
            columns: [],
            created_at_ms: 300n,
            updated_at_ms: 300n,
          },
        ],
      }),
      dataset_download: () => ({ version: 1, saved: true, file_name: "sales.csv" }),
    });
    render(<DatasetsView />);
    await waitFor(() => expect(screen.getByText("Sales Q3")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: "Save Sales Q3" }));

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("dataset_download", { datasetId: "d-1", fileId: "f-1" }),
    );
    // The drawer is for choosing among files; one file needs no choice.
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("rejects details for a different dataset before saving a file", async () => {
    mockBoundary({
      dataset_list: () => listOf([SALES]),
      dataset_detail: () => ({
        version: 1,
        dataset: INVENTORY,
        uploads_enabled: true,
        files: [
          {
            file_id: "wrong-file",
            table_name: "inventory",
            row_count: 1n,
            column_count: 1,
            size_bytes: 10n,
            original_filename: "inventory.csv",
            columns: [],
            created_at_ms: 100n,
            updated_at_ms: 100n,
          },
        ],
      }),
      dataset_download: () => ({ version: 1, saved: true, file_name: "inventory.csv" }),
    });
    render(<DatasetsView />);
    await screen.findByText("Sales Q3");

    await userEvent.click(screen.getByRole("button", { name: "Save Sales Q3" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Save Sales Q3" })).toBeEnabled());

    expect(invokeMock.mock.calls.filter(([command]) => command === "dataset_download")).toHaveLength(0);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("allows only one row to resolve a native save at a time", async () => {
    let finishDetail!: (value: unknown) => void;
    const detailPending = new Promise((resolve) => {
      finishDetail = resolve;
    });
    mockBoundary({ dataset_list: () => listOf([SALES, INVENTORY]), dataset_detail: () => detailPending });
    render(<DatasetsView />);
    await screen.findByText("Sales Q3");
    const sales = screen.getByRole("button", { name: "Save Sales Q3" });
    const inventory = screen.getByRole("button", { name: "Save Inventory" });

    await userEvent.click(sales);
    inventory.click();

    expect(invokeMock.mock.calls.filter(([command]) => command === "dataset_detail")).toHaveLength(1);
    expect(sales).toBeDisabled();
    expect(inventory).toBeDisabled();
    finishDetail({ version: 1, dataset: SALES, uploads_enabled: true, files: [] });
    await waitFor(() => expect(inventory).toBeEnabled());
  });

  it("offers a retry on a failed transfer, with the reason the backend wrote", async () => {
    mockBoundary({ dataset_list: () => listOf([]), upload_retry: () => queueOf([]) });
    render(<DatasetsView />);
    await waitFor(() => expect(screen.getByText("No datasets yet")).toBeInTheDocument());

    applyUploadQueue(
      queueOf([
        {
          version: 1,
          id: "u-1",
          purpose: "persistent_dataset",
          state: "failed",
          bytes_sent: 0,
          size: 1000,
          display_name: "sales.csv",
          safe_error: "That file is larger than this workspace accepts.",
        },
      ]),
    );

    await waitFor(() =>
      expect(screen.getByText("That file is larger than this workspace accepts.")).toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: "Dismiss failed upload sales.csv" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(invokeMock).toHaveBeenCalledWith("upload_retry", { uploadId: "u-1" });
  });
});
