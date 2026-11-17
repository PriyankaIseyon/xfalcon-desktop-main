import type React from "react";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  DatasetPreview,
  DatasetUploadStartResult,
  StagedDatasetView,
  UploadProjectionV1,
  UploadQueueV1,
} from "../../lib/generated/contracts";
import { applyUploadQueue, resetUploadQueueForTests } from "../uploads/uploadQueueStore";
import { DatasetUploadDialog } from "./DatasetUploadDialog";

const invokeMock = vi.hoisted(() => vi.fn());
let queueRevision = 0;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  convertFileSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

function staged(overrides: Partial<StagedDatasetView> = {}): StagedDatasetView {
  return {
    version: 1,
    id: "staged-1",
    display_name: "sales.csv",
    size: 4096,
    format: "csv",
    worksheets: [],
    ...overrides,
  };
}

function started(queue = transfer(), uploadId = "up-1"): DatasetUploadStartResult {
  return { version: 1, upload_id: uploadId, queue };
}

function preview(overrides: Partial<DatasetPreview> = {}): DatasetPreview {
  return {
    version: 1,
    staged_id: "staged-1",
    worksheet_index: 0,
    header_row_index: 0,
    suggested_header_row: 0,
    columns: [
      { name: "region", inferred_type: "string" },
      { name: "revenue", inferred_type: "integer" },
    ],
    rows: [
      ["EMEA", "10"],
      ["APAC", null],
    ],
    header_candidates: [
      ["region", "revenue"],
      ["EMEA", "10"],
    ],
    sampled_rows: 2,
    truncated_rows: false,
    truncated_columns: false,
    truncated_cell: false,
    unavailable: null,
    recipe_available: true,
    ...overrides,
  };
}

function transfer(overrides: Partial<UploadProjectionV1> = {}): UploadQueueV1 {
  return {
    version: 1,
    revision: ++queueRevision,
    uploads: [
      {
        version: 1,
        id: "up-1",
        purpose: "persistent_dataset",
        state: "selected",
        bytes_sent: 0,
        size: 4096,
        display_name: "sales",
        safe_error: null,
        ...overrides,
      },
    ],
  };
}

/** Publishes a queue the way the backend's event does. */
function publish(overrides: Partial<UploadProjectionV1> = {}) {
  act(() => applyUploadQueue(transfer(overrides)));
}

function mockBoundary(handlers: Record<string, (args: Record<string, unknown>) => unknown>) {
  invokeMock.mockImplementation(async (command: string, args: Record<string, unknown>) => {
    const handler = handlers[command];
    if (!handler) throw new Error(`Unexpected invoke: ${command}`);
    return handler(args ?? {});
  });
}

const previewCalls = () =>
  invokeMock.mock.calls.filter(([command]) => command === "dataset_preview").map(([, args]) => args);

const uploadCall = () =>
  invokeMock.mock.calls.find(([command]) => command === "dataset_upload_start")?.[1] as
    { request: Record<string, unknown> } | undefined;

afterEach(cleanup);

/** Every prop the dialog needs, with only the ones a test cares about named. */
function props(overrides: Partial<React.ComponentProps<typeof DatasetUploadDialog>> = {}) {
  return {
    staged: staged(),
    existingNames: [] as string[],
    onClose: () => {},
    onChooseAnother: () => {},
    onTransferStarted: () => {},
    ...overrides,
  };
}

beforeEach(() => {
  invokeMock.mockReset();
  resetUploadQueueForTests();
  queueRevision = 0;
});

describe("the confirm step", () => {
  it("shows the rows before anything is sent", async () => {
    mockBoundary({ dataset_preview: () => preview() });
    render(<DatasetUploadDialog {...props({ staged: staged() })} />);

    await waitFor(() => expect(screen.getByText("EMEA")).toBeInTheDocument());
    expect(screen.getByRole("columnheader", { name: /region/ })).toBeInTheDocument();
    // An absent cell reads as a gap, not as an empty string: the difference
    // decides whether a remove-nulls step would drop the row.
    expect(screen.getByText("—")).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("dataset_upload_start", expect.anything());
  });

  it("reveals a long preview cell only when it is clipped", async () => {
    const longCell = "North America enterprise revenue ".repeat(12);
    mockBoundary({ dataset_preview: () => preview({ rows: [[longCell, "10"]] }) });
    render(<DatasetUploadDialog {...props()} />);
    const cell = await screen.findByText(longCell.trim());
    Object.defineProperty(cell, "clientWidth", { configurable: true, value: 120 });
    Object.defineProperty(cell, "scrollWidth", { configurable: true, value: 600 });

    fireEvent.pointerEnter(cell);

    expect(cell).toHaveAttribute("title", longCell);
  });

  it("says what the preview left out", async () => {
    mockBoundary({
      dataset_preview: () => preview({ truncated_rows: true, truncated_columns: true, truncated_cell: true }),
    });
    render(<DatasetUploadDialog {...props({ staged: staged() })} />);

    await waitFor(() => expect(screen.getByText(/first 2 rows/)).toBeInTheDocument());
    const meta = screen.getByText(/first 2 rows/).textContent ?? "";
    expect(meta).toContain("2 columns");
    expect(meta).toContain("some columns not shown");
    expect(meta).toContain("a long value shortened");
    // And the promise that the sample is not the upload.
    expect(screen.getByText(/The whole file uploads/)).toBeInTheDocument();
  });

  it("keeps the upload available when a file cannot be previewed", async () => {
    mockBoundary({
      dataset_preview: () => preview({ unavailable: "file_too_large", columns: [], rows: [], recipe_available: false }),
      dataset_upload_start: () => started(),
    });
    render(<DatasetUploadDialog {...props({ staged: staged() })} />);

    await waitFor(() => expect(screen.getByText(/too large to read here/)).toBeInTheDocument());
    const upload = screen.getByRole("button", { name: "Upload dataset" });
    expect(upload).toBeEnabled();
    await userEvent.click(upload);
    expect(uploadCall()).toBeDefined();
  });

  it("offers the worksheets a workbook holds and re-reads on a switch", async () => {
    mockBoundary({
      dataset_preview: (args) => preview({ worksheet_index: Number(args.worksheetIndex ?? 0) }),
    });
    render(
      <DatasetUploadDialog
        staged={staged({ display_name: "book.xlsx", format: "xlsx", worksheets: ["Q3", "Notes"] })}
        existingNames={[]}
        onChooseAnother={() => {}}
        onClose={() => {}}
        onTransferStarted={() => {}}
      />,
    );

    await waitFor(() => expect(screen.getByText("EMEA")).toBeInTheDocument());
    // Named by its field, not by its current option: a select announces which
    // field it is, and the chosen worksheet is its VALUE.
    const worksheet = screen.getByRole("button", { name: /^Worksheet/ });
    expect(worksheet).toHaveTextContent("Q3");
    await userEvent.click(worksheet);
    await userEvent.click(screen.getByRole("menuitemradio", { name: "Notes" }));

    await waitFor(() => expect(previewCalls().at(-1)?.worksheetIndex).toBe(1));
  });

  it("lets the header row be chosen, and re-reads for it", async () => {
    // Mirrors the backend: asked for no particular row, it answers with the one
    // it suggests, so the control opens on the suggestion rather than on row 1.
    mockBoundary({
      dataset_preview: (args) =>
        preview({
          header_row_index: args.headerRowIndex === null ? 1 : Number(args.headerRowIndex),
          suggested_header_row: 1,
        }),
    });
    render(<DatasetUploadDialog {...props({ staged: staged() })} />);

    await waitFor(() => expect(screen.getByText("EMEA")).toBeInTheDocument());
    // The backend suggested row 2, so that is what the control shows first.
    const headerRow = screen.getByRole("button", { name: /^Header row/ });
    expect(headerRow).toHaveTextContent("Row 2 — EMEA");
    await userEvent.click(headerRow);
    await userEvent.click(screen.getByRole("menuitemradio", { name: /Row 1 — region/ }));

    await waitFor(() => expect(previewCalls().at(-1)?.headerRowIndex).toBe(0));
  });

  it("re-reads the rows for every cleanup step, so the preview is the result", async () => {
    mockBoundary({ dataset_preview: () => preview() });
    render(<DatasetUploadDialog {...props({ staged: staged() })} />);
    await waitFor(() => expect(screen.getByText("EMEA")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: /^Filter:/ }));

    await waitFor(() => {
      const last = previewCalls().at(-1);
      expect(last?.recipe).toEqual({
        steps: [{ kind: "filter", column: "region", op: "equals", value: "" }],
      });
    });
  });

  it("names the step the backend refused instead of dropping it", async () => {
    mockBoundary({
      dataset_preview: (args) => {
        if (args.recipe !== null) {
          throw { version: 1, code: "recipe_invalid_step", step_index: 0 };
        }
        return preview();
      },
    });
    render(<DatasetUploadDialog {...props({ staged: staged() })} />);
    await waitFor(() => expect(screen.getByText("EMEA")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: /^Filter:/ }));
    await waitFor(() => expect(screen.getByText(/Step 1 cannot be applied/)).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Upload dataset" })).toBeDisabled();
  });

  it("keeps a stale preview visible but blocks upload when its refresh fails", async () => {
    mockBoundary({
      dataset_preview: (args) => {
        if (args.recipe !== null) throw { version: 1, code: "network", step_index: null };
        return preview();
      },
    });
    render(<DatasetUploadDialog {...props()} />);
    await screen.findByText("EMEA");

    await userEvent.click(screen.getByRole("button", { name: /^Filter:/ }));

    expect(await screen.findByText("The workspace could not be reached.")).toBeInTheDocument();
    expect(screen.getByText("EMEA")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Upload dataset" })).toBeDisabled();
  });

  it("sends the chosen worksheet and header row when there is no cleanup", async () => {
    mockBoundary({
      dataset_preview: () => preview({ header_row_index: 1 }),
      dataset_upload_start: () => started(),
    });
    render(
      <DatasetUploadDialog
        staged={staged({ display_name: "book.xlsx", format: "xlsx", worksheets: ["Q3", "Notes"] })}
        existingNames={[]}
        onChooseAnother={() => {}}
        onClose={() => {}}
        onTransferStarted={() => {}}
      />,
    );
    await waitFor(() => expect(screen.getByText("EMEA")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: "Upload dataset" }));
    await waitFor(() => expect(uploadCall()).toBeDefined());
    expect(uploadCall()?.request).toMatchObject({
      staged_id: "staged-1",
      worksheet_index: 0,
      header_row_index: 1,
      recipe: null,
    });
  });

  it("sends the reviewed worksheet and header when a cleanup must be materialized", async () => {
    mockBoundary({
      dataset_preview: () => preview({ header_row_index: 1, suggested_header_row: 1 }),
      dataset_upload_start: () => started(),
    });
    render(
      <DatasetUploadDialog
        staged={staged({ display_name: "book.xlsx", format: "xlsx", worksheets: ["Q3"] })}
        existingNames={[]}
        onChooseAnother={() => {}}
        onClose={() => {}}
        onTransferStarted={() => {}}
      />,
    );
    await waitFor(() => expect(screen.getByText("EMEA")).toBeInTheDocument());
    await userEvent.click(screen.getByRole("button", { name: /^Filter:/ }));
    await waitFor(() => expect(previewCalls().at(-1)?.recipe).not.toBeNull());

    await waitFor(() => expect(screen.getByRole("button", { name: "Upload dataset" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "Upload dataset" }));
    await waitFor(() => expect(uploadCall()).toBeDefined());
    const request = uploadCall()?.request as Record<string, unknown>;
    expect(request.recipe).not.toBeNull();
    // The backend uses these to materialize the recipe, then drops them from
    // the later ingest request for the cleaned CSV.
    expect(request.worksheet_index).toBe(0);
    expect(request.header_row_index).toBe(1);
  });

  it("names the dataset after the file without its extension, and sends what was typed", async () => {
    mockBoundary({
      dataset_preview: () => preview(),
      dataset_upload_start: () => started(),
    });
    render(<DatasetUploadDialog {...props({ staged: staged() })} />);
    await waitFor(() => expect(screen.getByText("EMEA")).toBeInTheDocument());

    const name = screen.getByLabelText("Dataset name");
    // `sales.csv` reads as a file in a list of datasets; the dataset is `sales`.
    expect(name).toHaveValue("sales");

    await userEvent.clear(name);
    // Nothing to upload under no name — the file's own name is a fallback the
    // backend keeps, not a silent substitution the surface makes.
    expect(screen.getByRole("button", { name: "Upload dataset" })).toBeDisabled();

    await userEvent.type(name, "  Q3 actuals  ");
    await waitFor(() => expect(screen.getByRole("button", { name: "Upload dataset" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "Upload dataset" }));

    await waitFor(() => expect(uploadCall()).toBeDefined());
    expect(uploadCall()?.request.display_name).toBe("Q3 actuals");
  });

  it("still lets the worksheet and header row be set when there is no preview", async () => {
    // A file above the read ceiling uploads unshaped, and DA reads it with the
    // indices this sends. Without these there was no way to say "the data is on
    // the second sheet" for exactly the files that most often need it.
    mockBoundary({
      dataset_preview: () =>
        preview({
          unavailable: "file_too_large",
          columns: [],
          rows: [],
          header_candidates: [],
          recipe_available: false,
        }),
      dataset_upload_start: () => started(),
    });
    render(
      <DatasetUploadDialog
        {...props({ staged: staged({ display_name: "big.xlsx", format: "xlsx", worksheets: ["Q3", "Notes"] }) })}
      />,
    );
    await waitFor(() => expect(screen.getByText(/too large to read here/)).toBeInTheDocument());

    // Counted from one on screen, from zero on the wire: nobody calls the first
    // worksheet "worksheet 0".
    await userEvent.clear(screen.getByLabelText("Worksheet number"));
    await userEvent.type(screen.getByLabelText("Worksheet number"), "2");
    await userEvent.clear(screen.getByLabelText("Header row number"));
    await userEvent.type(screen.getByLabelText("Header row number"), "3");
    await waitFor(() => expect(screen.getByRole("button", { name: "Upload dataset" })).toBeEnabled());
    await userEvent.click(screen.getByRole("button", { name: "Upload dataset" }));

    await waitFor(() => expect(uploadCall()).toBeDefined());
    expect(uploadCall()?.request).toMatchObject({ worksheet_index: 1, header_row_index: 2 });
  });

  it("does not silently turn an invalid fallback row into row one", async () => {
    mockBoundary({
      dataset_preview: () =>
        preview({
          unavailable: "file_too_large",
          columns: [],
          rows: [],
          header_candidates: [],
          recipe_available: false,
        }),
      dataset_upload_start: () => started(),
    });
    render(<DatasetUploadDialog {...props({ staged: staged() })} />);
    await screen.findByText(/too large to read here/);
    const header = screen.getByLabelText("Header row number");

    await userEvent.clear(header);
    await userEvent.type(header, "-2x");

    expect(header).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText("Enter a positive whole number.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Upload dataset" })).toBeDisabled();
    expect(uploadCall()).toBeUndefined();
  });

  it("hands a different file back to the picker instead of closing on nothing", async () => {
    mockBoundary({ dataset_preview: () => preview(), dataset_stage_discard: () => null });
    const onChooseAnother = vi.fn();
    render(<DatasetUploadDialog {...props({ onChooseAnother })} />);
    await waitFor(() => expect(screen.getByText("EMEA")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: "Choose different file" }));
    await waitFor(() => expect(onChooseAnother).toHaveBeenCalled());
    expect(invokeMock).toHaveBeenCalledWith("dataset_stage_discard", { stagedId: "staged-1" });
  });

  it("keeps a failed stage release in the dialog and lets the person retry it", async () => {
    let attempts = 0;
    mockBoundary({
      dataset_preview: () => preview(),
      dataset_stage_discard: () => {
        attempts += 1;
        if (attempts === 1) throw { version: 1, code: "storage", step_index: null };
      },
    });
    const onChooseAnother = vi.fn();
    render(<DatasetUploadDialog {...props({ onChooseAnother })} />);
    await screen.findByText("EMEA");

    await userEvent.click(screen.getByRole("button", { name: "Choose different file" }));
    expect(await screen.findByText("That file could not be written.")).toBeInTheDocument();
    expect(onChooseAnother).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Choose different file" }));
    await waitFor(() => expect(onChooseAnother).toHaveBeenCalledOnce());
  });

  it("warns when the name is one the workspace already holds", async () => {
    mockBoundary({ dataset_preview: () => preview() });
    render(<DatasetUploadDialog {...props({ staged: staged(), existingNames: ["Sales Q3", "sales"] })} />);
    await waitFor(() => expect(screen.getByText("EMEA")).toBeInTheDocument());

    // Matched without regard to case, the way a person reads two rows of a list.
    expect(screen.getByText(/already in this workspace/)).toBeInTheDocument();
    // Still allowed: two datasets may share a name, and only the person knows
    // whether this one is a replacement.
    expect(screen.getByRole("button", { name: "Upload dataset" })).toBeEnabled();

    await userEvent.clear(screen.getByLabelText("Dataset name"));
    await userEvent.type(screen.getByLabelText("Dataset name"), "Sales Q4");
    expect(screen.queryByText(/already in this workspace/)).not.toBeInTheDocument();
  });

  it("releases the staged file when the person cancels", async () => {
    mockBoundary({
      dataset_preview: () => preview(),
      dataset_stage_discard: () => null,
    });
    const onClose = vi.fn();
    render(<DatasetUploadDialog {...props({ onClose })} />);
    await waitFor(() => expect(screen.getByText("EMEA")).toBeInTheDocument());

    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }));
    expect(invokeMock).toHaveBeenCalledWith("dataset_stage_discard", { stagedId: "staged-1" });
    expect(onClose).toHaveBeenCalled();
  });
});

/**
 * The transfer, watched where it was started.
 *
 * The dialog holds no progress of its own — it reads the shared queue — so these
 * publish queue snapshots the way the backend's event does.
 */
describe("the transfer", () => {
  const reachConfirm = async () => {
    mockBoundary({
      dataset_preview: () => preview(),
      dataset_upload_start: () => started(),
      upload_cancel: () => transfer({ state: "cancelled" }),
      upload_retry: () => transfer({ state: "selected" }),
    });
    await waitFor(() => expect(screen.getByText("EMEA")).toBeInTheDocument());
    await userEvent.click(screen.getByRole("button", { name: "Upload dataset" }));
  };

  it("keeps the dialog open on the transfer instead of returning to the list", async () => {
    const onTransferStarted = vi.fn();
    const onClose = vi.fn();
    render(<DatasetUploadDialog {...props({ onClose, onTransferStarted })} />);
    await reachConfirm();

    // The dialog stays; the destination is told which transfer it holds so one
    // upload is never two bars.
    await waitFor(() => expect(onTransferStarted).toHaveBeenCalledWith("up-1"));
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    publish({ state: "uploading", bytes_sent: 1024 });
    // Once over the bar and once on the button that is doing it.
    expect(await screen.findAllByText("Uploading…")).toHaveLength(2);
    expect(screen.getByText("25%")).toBeInTheDocument();
    // What was being confirmed is settled: the rows and the name are gone, and
    // the file they belong to is still named.
    expect(screen.queryByLabelText("Dataset name")).not.toBeInTheDocument();
    expect(screen.queryByText("EMEA")).not.toBeInTheDocument();
    expect(screen.getByText("sales.csv")).toBeInTheDocument();
  });

  it("follows the transfer this command added, not an older selected transfer", async () => {
    const old = transfer({ id: "old", state: "selected" }).uploads[0];
    act(() => applyUploadQueue({ version: 1, revision: 1, uploads: [old] }));
    mockBoundary({
      dataset_preview: () => preview(),
      dataset_upload_start: () =>
        started(
          {
            version: 1,
            revision: 2,
            uploads: [{ ...old, id: "new", state: "preparing", display_name: "sales" }, old],
          },
          "new",
        ),
    });
    const onTransferStarted = vi.fn();
    render(<DatasetUploadDialog {...props({ onTransferStarted })} />);
    await screen.findByText("EMEA");

    await userEvent.click(screen.getByRole("button", { name: "Upload dataset" }));

    await waitFor(() => expect(onTransferStarted).toHaveBeenCalledWith("new"));
    expect(onTransferStarted).not.toHaveBeenCalledWith("old");
  });

  it("counts no percentage for work whose bytes nobody can see", async () => {
    render(<DatasetUploadDialog {...props()} />);
    await reachConfirm();

    // A bar frozen at 0% while the workspace ingests the file reads as a stall.
    publish({ state: "finalizing", bytes_sent: 4096 });
    expect(await screen.findAllByText("Processing…")).toHaveLength(2);
    expect(screen.queryByText("100%")).not.toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Upload progress" })).not.toHaveAttribute("aria-valuenow");
  });

  it("says the dataset landed, and closes itself once it has been read", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    // jsdom's rAF is tied to its own clock; drive it off the fake timers so the
    // countdown can be advanced rather than waited out.
    vi.stubGlobal(
      "requestAnimationFrame",
      (callback: FrameRequestCallback) => setTimeout(() => callback(Date.now()), 16) as unknown as number,
    );
    vi.stubGlobal("cancelAnimationFrame", (handle: number) => clearTimeout(handle));
    try {
      const onClose = vi.fn();
      render(<DatasetUploadDialog {...props({ onClose })} />);
      await reachConfirm();

      publish({ state: "ready", bytes_sent: 4096 });
      expect(await screen.findByText("Ready to query in chat.")).toBeInTheDocument();
      expect(screen.getByRole("heading", { name: "Upload complete" })).toBeInTheDocument();
      expect(screen.getByText("Closing in 8s")).toBeInTheDocument();
      expect(onClose).not.toHaveBeenCalled();

      await act(async () => {
        vi.advanceTimersByTime(8100);
      });
      expect(onClose).toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
      vi.useRealTimers();
    }
  });

  it("closes on the person's word without waiting out the countdown", async () => {
    const onClose = vi.fn();
    render(<DatasetUploadDialog {...props({ onClose })} />);
    await reachConfirm();

    publish({ state: "ready", bytes_sent: 4096 });
    await userEvent.click(await screen.findByRole("button", { name: "Close now" }));
    expect(onClose).toHaveBeenCalled();
    // The staged file went with the transfer; releasing it now would be
    // releasing something the upload already consumed.
    expect(invokeMock).not.toHaveBeenCalledWith("dataset_stage_discard", expect.anything());
  });

  it("ends the transfer when the person cancels it, and goes with it", async () => {
    const onClose = vi.fn();
    render(<DatasetUploadDialog {...props({ onClose })} />);
    await reachConfirm();
    publish({ state: "uploading", bytes_sent: 1024 });

    await userEvent.click(await screen.findByRole("button", { name: "Cancel upload" }));
    expect(invokeMock).toHaveBeenCalledWith("upload_cancel", { uploadId: "up-1" });

    // The queue is what ends it; the dialog leaves when it has.
    publish({ state: "cancelled", bytes_sent: 1024 });
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("keeps a failed transfer on screen with its reason and its retry", async () => {
    render(<DatasetUploadDialog {...props()} />);
    await reachConfirm();

    publish({ state: "failed", bytes_sent: 1024, safe_error: "The workspace refused the file." });
    expect(await screen.findByText("The workspace refused the file.")).toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(invokeMock).toHaveBeenCalledWith("upload_retry", { uploadId: "up-1" });
  });

  it("leaves the staged file alone when the dialog is closed mid-transfer", async () => {
    // Closing hands the bar back to the destination. It does not end the
    // upload, and the staged entry it consumed is already gone.
    const onClose = vi.fn();
    render(<DatasetUploadDialog {...props({ onClose })} />);
    await reachConfirm();
    publish({ state: "uploading", bytes_sent: 1024 });

    await userEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }));
    expect(invokeMock).not.toHaveBeenCalledWith("dataset_stage_discard", expect.anything());
    expect(invokeMock).not.toHaveBeenCalledWith("upload_cancel", expect.anything());
    expect(onClose).toHaveBeenCalled();
  });
});
