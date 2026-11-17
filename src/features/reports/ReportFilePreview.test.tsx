import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReportView } from "../../lib/generated/contracts";
import { ReportFilePreview } from "./ReportFilePreview";

const reportOfficePreviewMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({ reportOfficePreview: reportOfficePreviewMock }));
vi.mock("../projects/OfficePdfViewer", () => ({
  OfficePdfViewer: ({ data, name }: { data: Uint8Array; name: string }) => (
    <div data-testid="office-pdf-preview">
      {name}:{data[0]}
    </div>
  ),
}));

const PDF = (first = 37) => ({ data: new Uint8Array([first, 80, 68, 70]), renderId: "render-1" });

function report(overrides: Partial<ReportView> = {}): ReportView {
  return {
    version: 1,
    id: "report-5",
    title: "Quarterly workbook",
    kind: "workbook",
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
    refreshable: true,
    can_refresh: true,
    can_manage: true,
    can_delete: true,
    page_count: 0,
    dataset_count: 1,
    source_conversation_id: null,
    ...overrides,
  } as ReportView;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

beforeEach(() => {
  reportOfficePreviewMock.mockReset();
});
afterEach(cleanup);

describe("ReportFilePreview", () => {
  it("makes the PDF as the report opens and shows it", async () => {
    reportOfficePreviewMock.mockResolvedValue(PDF());
    render(<ReportFilePreview exporting={false} onDownload={vi.fn()} report={report()} />);

    expect(screen.getByText("Making the PDF preview…")).toBeInTheDocument();
    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent("Quarterly workbook.xlsx:37");
    expect(reportOfficePreviewMock).toHaveBeenCalledTimes(1);
    expect(reportOfficePreviewMock).toHaveBeenCalledWith("report-5", expect.any(AbortSignal));
  });

  it("names a deck as a presentation file", async () => {
    reportOfficePreviewMock.mockResolvedValue(PDF());
    render(
      <ReportFilePreview
        exporting={false}
        onDownload={vi.fn()}
        report={report({ kind: "presentation", title: "Board deck" })}
      />,
    );
    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent("Board deck.pptx:37");
  });

  it("says why the preview failed, and still offers the download and another try", async () => {
    reportOfficePreviewMock
      .mockRejectedValueOnce(
        "This preview needs Microsoft Office, LibreOffice or WPS Office, and none of them was found.",
      )
      .mockResolvedValueOnce(PDF(38));
    const onDownload = vi.fn();
    const user = userEvent.setup();
    render(<ReportFilePreview exporting={false} onDownload={onDownload} report={report()} />);

    expect(
      await screen.findByText(/This preview needs Microsoft Office, LibreOffice or WPS Office/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^Download Quarterly workbook as Excel/ }));
    expect(onDownload).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent("Quarterly workbook.xlsx:38");
    expect(reportOfficePreviewMock).toHaveBeenCalledTimes(2);
  });

  it("falls back to a plain sentence when the failure carries none", async () => {
    reportOfficePreviewMock.mockRejectedValueOnce({ unexpected: true });
    render(<ReportFilePreview exporting={false} onDownload={vi.fn()} report={report()} />);
    expect(await screen.findByText(/The preview could not be made\./)).toBeInTheDocument();
  });

  it("stops on Cancel, ignores a PDF that arrives late, and starts again on request", async () => {
    const first = deferred<ReturnType<typeof PDF>>();
    reportOfficePreviewMock.mockReturnValueOnce(first.promise).mockResolvedValueOnce(PDF(39));
    const user = userEvent.setup();
    render(<ReportFilePreview exporting={false} onDownload={vi.fn()} report={report()} />);

    await user.click(await screen.findByRole("button", { name: "Cancel" }));
    const signal = reportOfficePreviewMock.mock.calls[0][1] as AbortSignal;
    expect(signal.aborted).toBe(true);

    first.resolve(PDF(40));
    await waitFor(() => expect(screen.getByRole("button", { name: "Preview as PDF" })).toBeInTheDocument());
    expect(screen.queryByTestId("office-pdf-preview")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Preview as PDF" }));
    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent("Quarterly workbook.xlsx:39");
  });

  it("makes a new preview when a refresh changes the report, and none when it does not", async () => {
    reportOfficePreviewMock.mockResolvedValueOnce(PDF(41)).mockResolvedValueOnce(PDF(42));
    const view = render(<ReportFilePreview exporting={false} onDownload={vi.fn()} report={report()} />);
    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent(":41");

    // The same report again: nothing is made.
    view.rerender(<ReportFilePreview exporting={true} onDownload={vi.fn()} report={report()} />);
    expect(reportOfficePreviewMock).toHaveBeenCalledTimes(1);

    view.rerender(
      <ReportFilePreview
        exporting={false}
        onDownload={vi.fn()}
        report={report({ updated_at_ms: 1_783_600_000_000n })}
      />,
    );
    await waitFor(() => expect(screen.getByTestId("office-pdf-preview")).toHaveTextContent(":42"));
    expect(reportOfficePreviewMock).toHaveBeenCalledTimes(2);
  });

  it("abandons the preview when the viewer closes", async () => {
    reportOfficePreviewMock.mockReturnValue(new Promise(() => undefined));
    const view = render(<ReportFilePreview exporting={false} onDownload={vi.fn()} report={report()} />);
    const signal = reportOfficePreviewMock.mock.calls[0][1] as AbortSignal;
    view.unmount();
    expect(signal.aborted).toBe(true);
  });
});
