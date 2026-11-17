import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantMessage } from "../chat/AssistantMessage";
import { getToasts, resetToastsForTests } from "../../ui/toastStore";
import { type ProjectReportDestination, ReportDestinationContext } from "./reportDestination";

const saveToProjectMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  previewDocumentRegister: vi.fn(async () => "beef".repeat(16)),
  previewDocumentSrc: (id: string) => `http://xf-preview.localhost/${id}`,
  reportSaveToProject: saveToProjectMock,
}));

/** Each document is over the 100-byte span floor, so the scanner keeps it. */
function document(marker: string): string {
  return `<!DOCTYPE html><html><head><title>${marker}</title></head><body><div id="${marker}">Bookings by region for the third quarter, charted</div></body></html>`;
}

const CONVERSATION = "conv-1";
const MESSAGE = "3f2a6c48-1e5b-4d7a-9c31-0b8e5f6a2d14";
const PROJECT_ID = "7d4b2a10-5c3e-4f61-8a9b-2e1d0c9f8a77";
const EXTERNAL_WARNING = "This report references a script it cannot load, so part of it will be missing.";
const SAVE_ALPHA = /^Saved? to project: alpha \(Finance\)$/;

function projectFile(relativePath: string) {
  return { version: 1 as const, project_id: PROJECT_ID, relative_path: relativePath };
}

function destinationFor(conversationId: string, openFile: (path: string) => void = vi.fn()): ProjectReportDestination {
  return { conversationId, projectId: PROJECT_ID, projectName: "Finance", openFile };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

afterEach(cleanup);

beforeEach(() => {
  saveToProjectMock.mockReset();
  resetToastsForTests();
});

describe("Save to project, on an html preview card", () => {
  /** Renders a message inside a project chat. The destination is the open
   * conversation's binding, which is what the shell provides. */
  function renderInProject(
    options: {
      conversationId?: string;
      messageConversationId?: string | null;
      messageId?: string;
      streaming?: boolean;
      text?: string;
      openFile?: (path: string) => void;
    } = {},
  ) {
    const destination = destinationFor(options.conversationId ?? CONVERSATION, options.openFile);
    render(
      <ReportDestinationContext.Provider value={destination}>
        <AssistantMessage
          conversationId={options.messageConversationId === undefined ? CONVERSATION : options.messageConversationId}
          messageId={options.messageId ?? MESSAGE}
          streaming={options.streaming}
          text={options.text ?? `Here you go:\n\n\`\`\`html\n${document("alpha")}\n\`\`\``}
        />
      </ReportDestinationContext.Provider>,
    );
    return destination;
  }

  it("offers no save control in a conversation with no project", () => {
    render(
      <AssistantMessage
        conversationId={CONVERSATION}
        messageId={MESSAGE}
        text={`Here you go:\n\n\`\`\`html\n${document("alpha")}\n\`\`\``}
      />,
    );
    expect(screen.queryByRole("button", { name: /save/i })).not.toBeInTheDocument();
    expect(saveToProjectMock).not.toHaveBeenCalled();
  });

  it("offers Save to project in a project conversation", () => {
    renderInProject();
    const button = screen.getByRole("button", { name: SAVE_ALPHA });
    expect(button).toHaveTextContent("Save to project");
  });

  it("is absent while the answer is still streaming", () => {
    renderInProject({ streaming: true });
    expect(screen.queryByRole("button", { name: /Save to project: .* \(Finance\)/ })).not.toBeInTheDocument();
  });

  it("is absent for an artifact that is not an html document", () => {
    renderInProject({ text: "```sql\nSELECT 1;\n```" });
    expect(screen.queryByRole("button", { name: /Save to project: .* \(Finance\)/ })).not.toBeInTheDocument();
  });

  it.each(["stream-run-1", "local-42"])(
    "is absent on the synthetic id %s, which no stored row carries",
    (syntheticId) => {
      renderInProject({ messageId: syntheticId });
      expect(screen.queryByRole("button", { name: /Save to project: .* \(Finance\)/ })).not.toBeInTheDocument();
    },
  );

  it("is absent when the turn has no conversation to name as its source", () => {
    renderInProject({ messageConversationId: null });
    expect(screen.queryByRole("button", { name: /Save to project: .* \(Finance\)/ })).not.toBeInTheDocument();
  });

  it("names the project, asks for no description, and saves by identity into it", async () => {
    saveToProjectMock.mockResolvedValue({
      version: 1,
      status: "saved",
      file: projectFile("reports/alpha.html"),
      limit: null,
      external_reference: false,
    });
    const openFile = vi.fn();
    const user = userEvent.setup();
    renderInProject({ openFile });

    await user.click(screen.getByRole("button", { name: SAVE_ALPHA }));
    expect(screen.getByRole("dialog", { name: "Save to Finance" })).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: /Description/ })).not.toBeInTheDocument();
    expect(screen.getByText("Saved as an HTML file in the reports folder of Finance.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save to project" }));

    await waitFor(() => expect(saveToProjectMock).toHaveBeenCalledTimes(1));
    const args = saveToProjectMock.mock.calls[0];
    expect(args).toEqual([CONVERSATION, MESSAGE, 0, "alpha", false]);
    for (const argument of args) {
      expect(String(argument)).not.toContain("<!DOCTYPE html");
    }
    expect(await screen.findByText("Saved to project")).toBeInTheDocument();
    // The accessible name starts with the words on the button.
    expect(screen.getByRole("button", { name: "Saved to project: alpha (Finance)" })).toBeInTheDocument();

    const notice = getToasts().find((item) => item.text === "Saved to Finance as reports/alpha.html");
    expect(notice?.action?.label).toBe("Open file");
    notice?.action?.onSelect();
    expect(openFile).toHaveBeenCalledWith("reports/alpha.html");
  });

  it("uses trimmed Unicode character limits that match the native store", async () => {
    saveToProjectMock.mockResolvedValue({
      version: 1,
      status: "saved",
      file: projectFile("reports/emoji.html"),
      limit: null,
      external_reference: false,
    });
    const user = userEvent.setup();
    renderInProject({ text: `\`\`\`html\n${document("emoji")}\n\`\`\`` });
    await user.click(screen.getByRole("button", { name: "Save to project: emoji (Finance)" }));
    const name = screen.getByRole("textbox", { name: /Name/ });
    const valid = "😀".repeat(200);

    fireEvent.change(name, { target: { value: `${valid}   ` } });
    expect(screen.getByRole("button", { name: "Save to project" })).toBeEnabled();
    fireEvent.change(name, { target: { value: `${valid}😀` } });
    expect(screen.getByText("Name must be 200 characters or fewer.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save to project" })).toBeDisabled();

    fireEvent.change(name, { target: { value: `  ${valid}  ` } });
    await user.click(screen.getByRole("button", { name: "Save to project" }));
    await waitFor(() => expect(saveToProjectMock).toHaveBeenCalledTimes(1));
    expect(saveToProjectMock.mock.calls[0][3]).toBe(valid);
  });

  it("resolves the second of three documents in one message to ordinal 1", async () => {
    saveToProjectMock.mockResolvedValue({
      version: 1,
      status: "saved",
      file: projectFile("reports/beta.html"),
      limit: null,
      external_reference: false,
    });
    const user = userEvent.setup();
    const text = ["alpha", "beta", "gamma"]
      .map((marker) => `Chart:\n\n\`\`\`html\n${document(marker)}\n\`\`\``)
      .join("\n\n");
    renderInProject({ text });

    await user.click(screen.getByRole("button", { name: "Save to project: beta (Finance)" }));
    await user.click(screen.getByRole("button", { name: "Save to project" }));

    await waitFor(() => expect(saveToProjectMock).toHaveBeenCalledTimes(1));
    expect(saveToProjectMock.mock.calls[0][2]).toBe(1);
  });

  it("warns before the save when the dashboard loads something the frame cannot reach", async () => {
    const user = userEvent.setup();
    const html = document("alpha").replace(
      "<body>",
      '<script src="https://cdn.jsdelivr.net/npm/chart.js"></script><body>',
    );
    renderInProject({ text: `\`\`\`html\n${html}\n\`\`\`` });

    await user.click(screen.getByRole("button", { name: SAVE_ALPHA }));

    // Before the bytes are stored, which is the only point at which declining
    // is still possible.
    expect(screen.getByText(/loads a script or stylesheet from the internet/)).toBeInTheDocument();
    expect(saveToProjectMock).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(saveToProjectMock).not.toHaveBeenCalled();
  });

  it("offers the existing file or a deliberate copy for identical bytes", async () => {
    saveToProjectMock.mockResolvedValueOnce({
      version: 1,
      status: "duplicate",
      file: projectFile("reports/Q3.html"),
      limit: null,
      external_reference: false,
    });
    const openFile = vi.fn();
    const user = userEvent.setup();
    renderInProject({ openFile });

    await user.click(screen.getByRole("button", { name: SAVE_ALPHA }));
    await user.click(screen.getByRole("button", { name: "Save to project" }));
    expect(
      await screen.findByText("Already saved as “reports/Q3.html”. Open it, or save another copy."),
    ).toBeInTheDocument();

    saveToProjectMock.mockResolvedValueOnce({
      version: 1,
      status: "saved",
      file: projectFile("reports/alpha (2).html"),
      limit: null,
      external_reference: false,
    });
    await user.click(screen.getByRole("button", { name: "Save a copy" }));
    await waitFor(() => expect(saveToProjectMock).toHaveBeenCalledTimes(2));
    expect(saveToProjectMock.mock.calls[1]).toEqual([CONVERSATION, MESSAGE, 0, "alpha", true]);

    await user.click(screen.getByRole("button", { name: SAVE_ALPHA }));
    saveToProjectMock.mockResolvedValueOnce({
      version: 1,
      status: "duplicate",
      file: projectFile("reports/Q3.html"),
      limit: null,
      external_reference: false,
    });
    await user.click(screen.getByRole("button", { name: "Save to project" }));
    await user.click(await screen.findByRole("button", { name: "Open existing" }));
    expect(openFile).toHaveBeenCalledWith("reports/Q3.html");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("uses the duplicate prompt's primary action when Enter saves a copy", async () => {
    saveToProjectMock
      .mockResolvedValueOnce({
        version: 1,
        status: "duplicate",
        file: projectFile("reports/Q3.html"),
        limit: null,
        external_reference: false,
      })
      .mockResolvedValueOnce({
        version: 1,
        status: "saved",
        file: projectFile("reports/alpha (2).html"),
        limit: null,
        external_reference: false,
      });
    const user = userEvent.setup();
    renderInProject();

    await user.click(screen.getByRole("button", { name: SAVE_ALPHA }));
    await user.click(screen.getByRole("button", { name: "Save to project" }));
    const name = await screen.findByRole("textbox", { name: /Name/ });
    await user.type(name, "{Enter}");

    await waitFor(() => expect(saveToProjectMock).toHaveBeenCalledTimes(2));
    expect(saveToProjectMock.mock.calls[1][4]).toBe(true);
    expect(await screen.findByText("Saved to project")).toBeInTheDocument();
  });

  it.each([
    [
      { status: "not_in_project", file: null, limit: null },
      "This chat is no longer part of a project, so nothing was saved.",
    ],
    [
      { status: "too_large", file: null, limit: 67108864n },
      "This dashboard is larger than the 64 MB a project report can hold.",
    ],
    [
      { status: "not_found", file: null, limit: null },
      "This dashboard could not be found in the saved conversation. Reopen the chat and try again.",
    ],
  ])("says what the outcome %o found", async (outcome, sentence) => {
    saveToProjectMock.mockResolvedValue({ version: 1, external_reference: false, ...outcome });
    const user = userEvent.setup();
    renderInProject();

    await user.click(screen.getByRole("button", { name: SAVE_ALPHA }));
    await user.click(screen.getByRole("button", { name: "Save to project" }));
    expect(await screen.findByText(sentence)).toBeInTheDocument();
    expect(screen.queryByText("Saved to project")).not.toBeInTheDocument();
  });

  it("does not offer a save control for a different open conversation", () => {
    renderInProject({ conversationId: "another-conversation" });
    expect(screen.queryByRole("button", { name: /save/i })).not.toBeInTheDocument();
  });

  it("blocks duplicate saves and ignores a completion after its real consumer unmounts", async () => {
    const save = deferred<{
      version: 1;
      status: "saved";
      file: ReturnType<typeof projectFile>;
      limit: null;
      external_reference: false;
    }>();
    saveToProjectMock.mockReturnValue(save.promise);
    const user = userEvent.setup();
    const destination = destinationFor(CONVERSATION);
    const { unmount } = render(
      <ReportDestinationContext.Provider value={destination}>
        <AssistantMessage
          conversationId={CONVERSATION}
          messageId={MESSAGE}
          text={`\`\`\`html\n${document("alpha")}\n\`\`\``}
        />
      </ReportDestinationContext.Provider>,
    );

    await user.click(screen.getByRole("button", { name: SAVE_ALPHA }));
    await user.click(screen.getByRole("button", { name: "Save to project" }));
    const busySave = screen.getByRole("button", { name: "Save to project" });
    expect(busySave).toBeDisabled();
    expect(busySave).toHaveAttribute("aria-busy", "true");
    await user.click(busySave);
    expect(saveToProjectMock).toHaveBeenCalledTimes(1);

    unmount();
    await act(() =>
      save.resolve({
        version: 1,
        status: "saved",
        file: projectFile("reports/alpha.html"),
        limit: null,
        external_reference: false,
      }),
    );
    expect(getToasts().some((item) => item.text.startsWith("Saved to Finance"))).toBe(false);
  });

  it("keeps the destination the dialog named when the binding drops while it is open", async () => {
    saveToProjectMock.mockResolvedValue({
      version: 1,
      status: "not_in_project",
      file: null,
      limit: null,
      external_reference: false,
    });
    const destination = destinationFor(CONVERSATION);
    const card = (value: ProjectReportDestination | null) => (
      <ReportDestinationContext.Provider value={value}>
        <AssistantMessage
          conversationId={CONVERSATION}
          messageId={MESSAGE}
          text={`Here you go:\n\n\`\`\`html\n${document("alpha")}\n\`\`\``}
        />
      </ReportDestinationContext.Provider>
    );
    const user = userEvent.setup();
    const { rerender } = render(card(destination));

    await user.click(screen.getByRole("button", { name: SAVE_ALPHA }));
    rerender(card(null));
    expect(screen.getByRole("dialog", { name: "Save to Finance" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save to project" }));

    expect(
      await screen.findByText("This chat is no longer part of a project, so nothing was saved."),
    ).toBeInTheDocument();
    expect(saveToProjectMock).toHaveBeenCalledTimes(1);
  });

  it("still warns after a project save whose file references a script", async () => {
    saveToProjectMock.mockResolvedValue({
      version: 1,
      status: "saved",
      file: projectFile("reports/alpha.html"),
      limit: null,
      external_reference: true,
    });
    const user = userEvent.setup();
    renderInProject();

    await user.click(screen.getByRole("button", { name: SAVE_ALPHA }));
    await user.click(screen.getByRole("button", { name: "Save to project" }));

    expect(await screen.findByText("Saved to project")).toBeInTheDocument();
    expect(getToasts().some((item) => item.text === EXTERNAL_WARNING)).toBe(true);
  });

  it("says the folder could not be reached when the write is refused", async () => {
    saveToProjectMock.mockRejectedValue({ version: 1, code: "project_unavailable" });
    const user = userEvent.setup();
    renderInProject();

    await user.click(screen.getByRole("button", { name: SAVE_ALPHA }));
    await user.click(screen.getByRole("button", { name: "Save to project" }));
    expect(
      await screen.findByText(
        "The project folder could not be reached, so nothing was saved. Check that the folder still exists and try again.",
      ),
    ).toBeInTheDocument();
  });

  it("offers no way to write the document to disk", () => {
    renderInProject();
    for (const name of [/download/i, /export/i, /save to disk/i, /save a copy to/i]) {
      expect(screen.queryByRole("button", { name })).not.toBeInTheDocument();
    }
  });
});
