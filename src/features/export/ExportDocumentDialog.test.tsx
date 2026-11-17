import { StrictMode } from "react";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatToolCallView } from "../../lib/generated/contracts";
import { getToasts, resetToastsForTests } from "../../ui/toastStore";
import { ExportDocumentDialog } from "./ExportDocumentDialog";
import type { ExportTurn } from "./exportDocument";

const exportMock = vi.hoisted(() => vi.fn());
const pdfMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  documentExportSave: exportMock,
  documentExportPdf: pdfMock,
}));

function call(overrides: Partial<ChatToolCallView> = {}): ChatToolCallView {
  return {
    call_id: "call-1",
    name: "project_read_file",
    status: "answered",
    arguments: '{"path":"/etc/app.toml"}',
    arguments_truncated: false,
    result: null,
    result_truncated: false,
    step: null,
    ...overrides,
  };
}

const TURNS: ExportTurn[] = [
  { id: "s1", role: "system", text: "You are xFalcon.", toolCalls: [] },
  { id: "u1", role: "user", text: "Bookings by region?", toolCalls: [] },
  { id: "a1", role: "assistant", text: "EMEA led at 412.", toolCalls: [call()] },
];

function open(props: Partial<React.ComponentProps<typeof ExportDocumentDialog>> = {}) {
  const onClose = vi.fn();
  render(<ExportDocumentDialog open title="Bookings review" turns={TURNS} onClose={onClose} {...props} />);
  return { onClose };
}

afterEach(cleanup);

beforeEach(() => {
  exportMock.mockReset();
  pdfMock.mockReset();
  resetToastsForTests();
});

describe("what the export dialog offers when it opens", () => {
  it("selects every exportable turn and never lists the system prompt", () => {
    open();
    expect(screen.getByText("Turns (2 of 2)")).toBeTruthy();
    // Two turn rows plus the two payload toggles; the system turn is not a row.
    expect(screen.getAllByRole("checkbox")).toHaveLength(4);
    const rows = screen.getAllByRole("checkbox", { name: /question|answer/i });
    expect(rows).toHaveLength(2);
    expect(rows.every((row) => (row as HTMLInputElement).checked)).toBe(true);
    expect(screen.queryByText("You are xFalcon.")).toBeNull();
  });

  it("leaves both payload toggles off, so a first export carries neither", () => {
    open();
    const tools = screen.getByRole("checkbox", { name: /include tool calls/i });
    const reasoning = screen.getByRole("checkbox", { name: /include reasoning/i });
    expect((tools as HTMLInputElement).checked).toBe(false);
    expect((reasoning as HTMLInputElement).checked).toBe(false);
  });

  it("disables the reasoning toggle and says why when the run left none", () => {
    open();
    const reasoning = screen.getByRole("checkbox", { name: /include reasoning/i });
    expect((reasoning as HTMLInputElement).disabled).toBe(true);
    expect(screen.getByText(/reasoning is never saved with a conversation/i)).toBeTruthy();
  });

  it("enables the reasoning toggle once the live run has some", () => {
    open({ reasoning: ["Checking the schema."] });
    const reasoning = screen.getByRole("checkbox", { name: /include reasoning/i });
    expect((reasoning as HTMLInputElement).disabled).toBe(false);
  });

  it("disables the tool toggle and says why when no turn called a tool", () => {
    open({ turns: [{ id: "u1", role: "user", text: "Hello", toolCalls: [] }] });
    const tools = screen.getByRole("checkbox", { name: /include tool calls/i });
    expect((tools as HTMLInputElement).disabled).toBe(true);
    expect(screen.getByText(/no turn in this conversation called a tool/i)).toBeTruthy();
  });
});

describe("choosing what goes in", () => {
  it("clears and restores the whole selection from one control", async () => {
    const user = userEvent.setup();
    open();
    await user.click(screen.getByRole("button", { name: "Clear all" }));
    expect(screen.getByText("Turns (0 of 2)")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Select all" }));
    expect(screen.getByText("Turns (2 of 2)")).toBeTruthy();
  });

  it("exports an explicit document when no turns are selected", async () => {
    const user = userEvent.setup();
    exportMock.mockResolvedValue({ version: 1, saved: false, file_name: null });
    open();
    await user.click(screen.getByRole("button", { name: "Clear all" }));
    await user.click(screen.getByRole("radio", { name: /markdown/i }));
    await user.click(screen.getByRole("button", { name: /save markdown/i }));

    await waitFor(() => expect(exportMock).toHaveBeenCalledTimes(1));
    const [, body] = exportMock.mock.calls[0] as [string, string];
    expect(body).toContain("**Contents** 0 turns");
    expect(body).toContain("No turns were selected for this export.");
  });
});

describe("saving Markdown", () => {
  it("admits only one native save before the busy state renders", async () => {
    const user = userEvent.setup();
    exportMock.mockImplementation(() => new Promise(() => {}));
    open();

    await user.click(screen.getByRole("radio", { name: /markdown/i }));
    const save = screen.getByRole("button", { name: /save markdown/i });
    act(() => {
      save.click();
      save.click();
    });

    expect(exportMock).toHaveBeenCalledTimes(1);
  });

  it("sends the composed document and the conversation's name, and never a path", async () => {
    const user = userEvent.setup();
    exportMock.mockResolvedValue({ version: 1, saved: true, file_name: "Bookings review.md" });
    const { onClose } = open();

    await user.click(screen.getByRole("radio", { name: /markdown/i }));
    await user.click(screen.getByRole("button", { name: /save markdown/i }));

    await waitFor(() => expect(exportMock).toHaveBeenCalledTimes(1));
    const [suggestedName, body, format] = exportMock.mock.calls[0] as [string, string, string];
    // Three arguments and no fourth: there is no destination parameter to pass.
    expect(exportMock.mock.calls[0]).toHaveLength(3);
    expect(suggestedName).toBe("Bookings review");
    expect(format).toBe("markdown");
    expect(body).toContain("# Bookings review");
    expect(body).toContain("EMEA led at 412.");
    // The tool toggle was left alone, so its payload must not be in the file.
    expect(body).not.toContain("/etc/app.toml");
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(getToasts()[0]?.text).toBe("Saved Bookings review.md.");
  });

  it("includes tool payloads once the box is ticked", async () => {
    const user = userEvent.setup();
    exportMock.mockResolvedValue({ version: 1, saved: true, file_name: "out.md" });
    open();

    await user.click(screen.getByRole("radio", { name: /markdown/i }));
    await user.click(screen.getByRole("checkbox", { name: /include tool calls/i }));
    await user.click(screen.getByRole("button", { name: /save markdown/i }));

    await waitFor(() => expect(exportMock).toHaveBeenCalled());
    const [, body] = exportMock.mock.calls[0] as [string, string];
    expect(body).toContain("/etc/app.toml");
  });

  it("says nothing and stays open when the save dialog is dismissed", async () => {
    const user = userEvent.setup();
    exportMock.mockResolvedValue({ version: 1, saved: false, file_name: null });
    const { onClose } = open();

    await user.click(screen.getByRole("radio", { name: /markdown/i }));
    await user.click(screen.getByRole("button", { name: /save markdown/i }));

    await waitFor(() => expect(exportMock).toHaveBeenCalled());
    expect(onClose).not.toHaveBeenCalled();
    expect(getToasts()).toHaveLength(0);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("reports a refused write in the dialog rather than closing over it", async () => {
    const user = userEvent.setup();
    exportMock.mockRejectedValue(new Error("This document is too large to export."));
    const { onClose } = open();

    await user.click(screen.getByRole("radio", { name: /markdown/i }));
    await user.click(screen.getByRole("button", { name: /save markdown/i }));

    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("too large to export"));
    expect(onClose).not.toHaveBeenCalled();
  });

  it("ignores a Markdown result after its controlled dialog has closed", async () => {
    const user = userEvent.setup();
    let finish: (value: { version: 1; saved: boolean; file_name: string | null }) => void = () => {};
    exportMock.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const onClose = vi.fn();
    const view = render(<ExportDocumentDialog open title="Bookings review" turns={TURNS} onClose={onClose} />);

    await user.click(screen.getByRole("radio", { name: /markdown/i }));
    await user.click(screen.getByRole("button", { name: /save markdown/i }));
    await waitFor(() => expect(exportMock).toHaveBeenCalledTimes(1));

    view.rerender(<ExportDocumentDialog open={false} title="Bookings review" turns={TURNS} onClose={onClose} />);
    finish({ version: 1, saved: true, file_name: "late.md" });
    await Promise.resolve();

    expect(onClose).not.toHaveBeenCalled();
    expect(getToasts()).toHaveLength(0);
  });
});

describe("printing to PDF", () => {
  it("waits for bundled fonts to settle before asking the webview to write", async () => {
    const user = userEvent.setup();
    const originalFonts = Object.getOwnPropertyDescriptor(document, "fonts");
    let fontsReady: () => void = () => {};
    const ready = new Promise<void>((resolve) => {
      fontsReady = resolve;
    });
    Object.defineProperty(document, "fonts", { configurable: true, value: { ready } });
    pdfMock.mockResolvedValue({ version: 1, saved: false, file_name: null });

    try {
      open();
      await user.click(screen.getByRole("button", { name: /save pdf/i }));
      await Promise.resolve();
      expect(pdfMock).not.toHaveBeenCalled();

      fontsReady();
      await waitFor(() => expect(pdfMock).toHaveBeenCalledTimes(1), { timeout: 3000 });
    } finally {
      if (originalFonts) Object.defineProperty(document, "fonts", originalFonts);
      else Reflect.deleteProperty(document, "fonts");
    }
  });

  it("falls back to Markdown, with the selection intact, when the webview cannot write a PDF", async () => {
    const user = userEvent.setup();
    pdfMock.mockRejectedValue(new Error("This platform could not write a PDF."));
    const { onClose } = open();

    await user.click(screen.getByRole("button", { name: /save pdf/i }));

    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("could not write a PDF"), {
      timeout: 3000,
    });
    expect(pdfMock).toHaveBeenCalledTimes(1);
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByText("Turns (2 of 2)")).toBeTruthy();
    expect(screen.getByRole("button", { name: /save markdown/i })).toBeTruthy();
  });

  it("closes once the PDF has been written, and never calls window.print", async () => {
    const user = userEvent.setup();
    const print = vi.fn();
    vi.stubGlobal("print", print);
    pdfMock.mockResolvedValue({ version: 1, saved: true, file_name: "Bookings review.pdf" });
    const { onClose } = open();

    await user.click(screen.getByRole("button", { name: /save pdf/i }));

    await waitFor(() => expect(onClose).toHaveBeenCalled(), { timeout: 3000 });
    expect(pdfMock).toHaveBeenCalledWith("Bookings review");
    expect(pdfMock.mock.calls[0]).toHaveLength(1);
    expect(print).not.toHaveBeenCalled();
    expect(getToasts()[0]?.text).toBe("Saved Bookings review.pdf.");
    vi.unstubAllGlobals();
  });

  it("says nothing and stays open when the save dialog is dismissed", async () => {
    const user = userEvent.setup();
    pdfMock.mockResolvedValue({ version: 1, saved: false, file_name: null });
    const { onClose } = open();

    await user.click(screen.getByRole("button", { name: /save pdf/i }));

    await waitFor(() => expect(pdfMock).toHaveBeenCalled());
    expect(onClose).not.toHaveBeenCalled();
    expect(getToasts()).toHaveLength(0);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.getByRole("button", { name: /save pdf/i })).toBeTruthy();
  });

  it("prints the selected turns and omits the system prompt from the sheet", async () => {
    const user = userEvent.setup();
    let sheet = "";
    pdfMock.mockImplementation(async () => {
      sheet = document.querySelector(".xf-print-document")?.textContent ?? "";
      return { version: 1, saved: true, file_name: "out.pdf" };
    });
    open();

    await user.click(screen.getByRole("button", { name: /save pdf/i }));

    await waitFor(() => expect(sheet).toContain("EMEA led at 412."), { timeout: 3000 });
    expect(sheet).toContain("Bookings review");
    expect(sheet).toContain("Bookings by region?");
    expect(sheet).not.toContain("You are xFalcon.");
    // The tool toggle was untouched, so the sheet carries no payload either.
    expect(sheet).not.toContain("/etc/app.toml");
  });

  it("mounts all long prose, code, and wide-table content in the print sheet", async () => {
    const user = userEvent.setup();
    const columns = Array.from({ length: 12 }, (_, index) => `Column ${index + 1}`);
    const wideTable = [
      `| ${columns.join(" | ")} |`,
      `| ${columns.map(() => "---").join(" | ")} |`,
      `| ${columns.map((_, index) => `value-${index + 1}-${"x".repeat(80)}`).join(" | ")} |`,
    ].join("\n");
    const longCode = `\`\`\`text\n${"unbroken-code-token".repeat(300)}\n\`\`\``;
    const longProse = Array.from({ length: 120 }, (_, index) => `Paragraph ${index + 1}.`).join("\n\n");
    let sheet: Element | null = null;
    pdfMock.mockImplementation(async () => {
      sheet = document.querySelector(".xf-print-document");
      return { version: 1, saved: false, file_name: null };
    });
    open({
      turns: [{ id: "a-long", role: "assistant", text: `${longProse}\n\n${longCode}\n\n${wideTable}`, toolCalls: [] }],
    });

    await user.click(screen.getByRole("button", { name: /save pdf/i }));

    await waitFor(() => expect(sheet).not.toBeNull(), { timeout: 3000 });
    const mounted = sheet as Element | null;
    expect(mounted?.querySelectorAll(".xf-print-section > p")).toHaveLength(120);
    expect(mounted?.querySelector("pre")?.textContent).toContain("unbroken-code-token");
    expect(mounted?.querySelectorAll("th")).toHaveLength(12);
    expect(mounted?.querySelectorAll("td")).toHaveLength(12);
    expect(mounted?.textContent).toContain(`value-12-${"x".repeat(80)}`);
  });

  it("still opens the save dialog after the parent re-renders with a new onClose", async () => {
    const user = userEvent.setup();
    pdfMock.mockImplementation(() => new Promise(() => {}));
    const view = render(<ExportDocumentDialog open title="Bookings review" turns={TURNS} onClose={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: /save pdf/i }));
    view.rerender(<ExportDocumentDialog open title="Bookings review" turns={TURNS} onClose={vi.fn()} />);

    await waitFor(() => expect(pdfMock).toHaveBeenCalled());
  });

  it("still opens the save dialog under StrictMode's double mount", async () => {
    const user = userEvent.setup();
    pdfMock.mockResolvedValue({ version: 1, saved: false, file_name: null });
    render(
      <StrictMode>
        <ExportDocumentDialog open title="Bookings review" turns={TURNS} onClose={vi.fn()} />
      </StrictMode>,
    );

    await user.click(screen.getByRole("button", { name: /save pdf/i }));

    await waitFor(() => expect(pdfMock).toHaveBeenCalled());
  });

  it("lets Close dismiss the dialog while a PDF export is still running", async () => {
    const user = userEvent.setup();
    let finish: (value: { version: 1; saved: boolean; file_name: string | null }) => void = () => {};
    pdfMock.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const { onClose } = open();

    await user.click(screen.getByRole("button", { name: /save pdf/i }));
    await waitFor(() => expect(pdfMock).toHaveBeenCalled());

    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(1);

    finish({ version: 1, saved: true, file_name: "out.pdf" });
    await Promise.resolve();
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(getToasts()).toHaveLength(0);
  });

  it("lets Cancel dismiss the dialog while a PDF export is still running", async () => {
    const user = userEvent.setup();
    pdfMock.mockImplementation(() => new Promise(() => {}));
    const { onClose } = open();

    await user.click(screen.getByRole("button", { name: /save pdf/i }));
    await waitFor(() => expect(pdfMock).toHaveBeenCalled());

    const cancel = screen.getByRole("button", { name: "Cancel" });
    expect((cancel as HTMLButtonElement).disabled).toBe(false);
    await user.click(cancel);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
