import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DatasetDetail, DatasetView } from "../../lib/generated/contracts";
import { SchemaDrawer } from "./SchemaDrawer";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  convertFileSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

const SALES: DatasetView = {
  version: 1,
  id: "d-1",
  display_name: "Sales Q3",
  status: "ready",
  created_at_ms: 1_783_500_000_000n,
  updated_at_ms: 1_783_500_000_000n,
};

function detail(overrides: Partial<DatasetDetail["files"][number]> = {}): DatasetDetail {
  return {
    version: 1,
    dataset: SALES,
    uploads_enabled: true,
    files: [
      {
        file_id: "f-1",
        table_name: "sales_q3",
        row_count: 482_109,
        column_count: 3,
        size_bytes: 38 * 1024 * 1024,
        original_filename: "sales.csv",
        columns: [
          { name: "region", inferred_type: "VARCHAR" },
          { name: "month", inferred_type: "VARCHAR" },
          { name: "revenue", inferred_type: "DOUBLE" },
        ],
        created_at_ms: 1_783_500_000_000n,
        updated_at_ms: 1_783_500_000_000n,
        ...overrides,
      },
    ],
  };
}

afterEach(cleanup);

beforeEach(() => {
  invokeMock.mockReset();
});

describe("the schema drawer", () => {
  it("reads a dataset's files and their columns in the server's order", async () => {
    invokeMock.mockResolvedValue(detail());
    render(<SchemaDrawer dataset={SALES} onClose={() => {}} onDownload={async () => true} />);

    await waitFor(() => expect(screen.getByText("sales_q3")).toBeInTheDocument());
    expect(screen.getByText(/482,109 rows/)).toBeInTheDocument();
    expect(screen.getByText(/38 MB/)).toBeInTheDocument();
    expect(screen.getByText(/from sales.csv/)).toBeInTheDocument();

    const columns = screen.getAllByRole("listitem");
    for (const [index, name] of ["region", "month", "revenue"].entries()) {
      expect(within(columns[index]).getByText(name)).toBeInTheDocument();
    }
    expect(screen.getByLabelText("DOUBLE (Number)")).toBeInTheDocument();
    expect(screen.getAllByLabelText("VARCHAR (Text)")).toHaveLength(2);
  });

  it("filters columns by name or exact type without changing their order or type labels", async () => {
    invokeMock.mockResolvedValue(
      detail({
        columns: [
          { name: "calendar_date", inferred_type: "date32[day]" },
          { name: "region", inferred_type: "string" },
          { name: "count", inferred_type: "int64" },
          { name: "enabled", inferred_type: "bool" },
          { name: "metadata", inferred_type: "struct<value: string>" },
        ],
      }),
    );
    const user = userEvent.setup();
    render(<SchemaDrawer dataset={SALES} onClose={() => {}} onDownload={async () => true} />);
    const search = await screen.findByRole("searchbox", { name: "Search columns in sales_q3" });
    expect(screen.getByLabelText("date32[day] (Date / time)")).toBeInTheDocument();
    expect(screen.getByLabelText("bool (Boolean)")).toBeInTheDocument();
    expect(screen.getByLabelText("struct<value: string> (Other)")).toBeInTheDocument();
    await user.type(search, "INT64");
    expect(screen.getAllByRole("listitem")).toHaveLength(1);
    expect(screen.getByText("count")).toBeInTheDocument();
    expect(screen.getByText("1 of 5 columns")).toBeInTheDocument();
    await user.clear(search);
    await user.type(search, "calendar");
    expect(screen.getByText("calendar_date")).toBeInTheDocument();
    await user.clear(search);
    await user.type(search, "missing");
    expect(screen.getByText("No columns match “missing”.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Clear search" }));
    expect(screen.getAllByRole("listitem")).toHaveLength(5);
  });

  it("keeps an unrecorded count distinct from zero", async () => {
    invokeMock.mockResolvedValue(detail({ row_count: null, column_count: null, size_bytes: null }));
    render(<SchemaDrawer dataset={SALES} onClose={() => {}} onDownload={async () => true} />);

    await waitFor(() => expect(screen.getByText("sales_q3")).toBeInTheDocument());
    // Not "0 rows": a table of no rows and a table whose count was never
    // written are different facts.
    expect(screen.getByText(/Unknown rows · Unknown columns · Unknown/)).toBeInTheDocument();
  });

  it("hands the download to the backend by identity", async () => {
    invokeMock.mockResolvedValue(detail());
    const onDownload = vi.fn().mockResolvedValue(true);
    render(<SchemaDrawer dataset={SALES} onClose={() => {}} onDownload={onDownload} />);

    await waitFor(() => expect(screen.getByText("sales_q3")).toBeInTheDocument());
    await userEvent.click(screen.getByRole("button", { name: /Save a copy/ }));

    expect(onDownload).toHaveBeenCalledWith(SALES, expect.objectContaining({ file_id: "f-1" }));
  });

  it("blocks downloads when the workspace upload source is turned off", async () => {
    invokeMock.mockResolvedValue({ ...detail(), uploads_enabled: false });
    render(<SchemaDrawer dataset={SALES} onClose={() => {}} onDownload={async () => true} />);

    await waitFor(() => expect(screen.getByText("sales_q3")).toBeInTheDocument());
    expect(screen.getByRole("button", { name: /Save a copy/ })).toBeDisabled();
  });

  it("reports a schema it could not read", async () => {
    invokeMock.mockRejectedValue({ version: 1, code: "not_found", step_index: null });
    render(<SchemaDrawer dataset={SALES} onClose={() => {}} onDownload={async () => true} />);

    await waitFor(() => expect(screen.getByText("That dataset is no longer here.")).toBeInTheDocument());
  });

  it("rejects details for a different dataset", async () => {
    invokeMock.mockResolvedValue({ ...detail(), dataset: { ...SALES, id: "d-other" } });
    render(<SchemaDrawer dataset={SALES} onClose={() => {}} onDownload={async () => true} />);

    expect(await screen.findByText("The dataset details did not match the selected dataset.")).toBeInTheDocument();
    expect(screen.queryByText("sales_q3")).not.toBeInTheDocument();
  });

  it("retries a schema load from the error that owns it", async () => {
    invokeMock.mockRejectedValueOnce({ version: 1, code: "network", step_index: null }).mockResolvedValueOnce(detail());
    render(<SchemaDrawer dataset={SALES} onClose={() => {}} onDownload={async () => true} />);
    await screen.findByText("The workspace could not be reached.");

    await userEvent.click(screen.getByRole("button", { name: "Try again" }));

    expect(await screen.findByText("sales_q3")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledTimes(2);
  });

  it("allows only one native save dialog at a time", async () => {
    let finish!: () => void;
    const saving = new Promise<void>((resolve) => {
      finish = resolve;
    });
    const second = { ...detail().files[0], file_id: "f-2", table_name: "archive" };
    invokeMock.mockResolvedValue({ ...detail(), files: [detail().files[0], second] });
    const onDownload = vi.fn(() => saving.then(() => true));
    render(<SchemaDrawer dataset={SALES} onClose={() => {}} onDownload={onDownload} />);
    const buttons = await screen.findAllByRole("button", { name: /Save a copy/ });

    await userEvent.click(buttons[0]);
    expect(buttons[0]).toBeDisabled();
    expect(buttons[1]).toBeDisabled();
    buttons[1].click();
    expect(onDownload).toHaveBeenCalledOnce();

    finish();
    await waitFor(() => expect(buttons[1]).toBeEnabled());
  });

  it("says so when the workspace recorded no columns", async () => {
    invokeMock.mockResolvedValue(detail({ columns: [] }));
    render(<SchemaDrawer dataset={SALES} onClose={() => {}} onDownload={async () => true} />);

    await waitFor(() => expect(screen.getByText(/has not recorded this file's columns/)).toBeInTheDocument());
  });
});
