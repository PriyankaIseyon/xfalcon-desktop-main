import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getToasts, resetToastsForTests } from "../../ui/toastStore";
import { HtmlCanvas } from "./HtmlCanvas";
import { resetCanvasStore, type HtmlArtifact } from "./canvasStore";

const registerMock = vi.hoisted(() => vi.fn());
const saveMock = vi.hoisted(() => vi.fn());
const clipboard = vi.hoisted(() => vi.fn(async (text: string) => text));

vi.mock("../../lib/native", () => ({
  documentExportSave: saveMock,
  previewDocumentRegister: registerMock,
  previewDocumentSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));
vi.mock("../chat/clipboard", () => ({ writeClipboardText: clipboard }));

function artifact(overrides: Partial<HtmlArtifact> = {}): HtmlArtifact {
  return {
    id: "art-1",
    type: "html",
    title: "Dashboard",
    kind: "html",
    // Deliberately stale: the tab was opened earlier and the store may have
    // evicted this id since.
    documentId: "evicted-id",
    content: "<!DOCTYPE html><html><body>bookings</body></html>",
    ...overrides,
  };
}

afterEach(cleanup);

beforeEach(() => {
  registerMock.mockReset();
  saveMock.mockReset();
  clipboard.mockClear();
  resetToastsForTests();
  resetCanvasStore();
});

describe("getting the document on screen", () => {
  it("re-registers the source rather than trusting the id it was handed", async () => {
    registerMock.mockResolvedValue("fresh-id");
    render(<HtmlCanvas artifact={artifact()} />);

    await waitFor(() => expect(registerMock).toHaveBeenCalledTimes(1));
    expect(registerMock).toHaveBeenCalledWith("html", artifact().content);
    const frame = await screen.findByTitle("Dashboard");
    expect(frame).toHaveAttribute("inert");
    expect(frame).toHaveAttribute("aria-busy", "true");
    fireEvent.load(frame);
    expect(frame).not.toHaveAttribute("inert");
    // The fresh id, never the evicted one the artifact carried.
    expect(frame.getAttribute("src")).toBe("http://xf-preview.localhost/fresh-id");
  });

  it("replaces the frame on reload when registration returns the same preview id", async () => {
    const user = userEvent.setup();
    registerMock.mockResolvedValue("fresh-id");
    render(<HtmlCanvas artifact={artifact()} />);

    const firstFrame = await screen.findByTitle("Dashboard");
    fireEvent.load(firstFrame);
    await user.click(screen.getByRole("button", { name: "Reload" }));

    await waitFor(() => expect(registerMock).toHaveBeenCalledTimes(2));
    const reloadedFrame = await screen.findByTitle("Dashboard");
    expect(reloadedFrame).not.toBe(firstFrame);
    expect(reloadedFrame).toHaveAttribute("inert");
    fireEvent.load(reloadedFrame);
    expect(reloadedFrame).not.toHaveAttribute("inert");
  });

  it("registers an svg under its own kind, not as html", async () => {
    registerMock.mockResolvedValue("svg-id");
    render(<HtmlCanvas artifact={artifact({ kind: "svg", content: "<svg></svg>" })} />);

    await waitFor(() => expect(registerMock).toHaveBeenCalledWith("svg", "<svg></svg>"));
  });

  it("keeps the isolation contract its two sibling frames carry", async () => {
    registerMock.mockResolvedValue("fresh-id");
    render(<HtmlCanvas artifact={artifact()} />);

    const frame = await screen.findByTitle("Dashboard");
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
    // Never `allow-same-origin`: on its own origin the document would otherwise
    // reach app storage and IPC.
    expect(frame.getAttribute("sandbox")).not.toContain("allow-same-origin");
    expect(frame.getAttribute("referrerpolicy")).toBe("no-referrer");
  });
});

describe("when registration does not work", () => {
  it("retries once before saying anything, since the first failure is usually transient", async () => {
    registerMock.mockRejectedValueOnce(new Error("busy")).mockResolvedValueOnce("second-try");
    render(<HtmlCanvas artifact={artifact()} />);

    await waitFor(() => expect(registerMock).toHaveBeenCalledTimes(2));
    expect(await screen.findByTitle("Dashboard")).toBeTruthy();
    expect(screen.queryByText("This document could not be opened.")).toBeNull();
  });

  it("reports a document it cannot open instead of showing a blank frame", async () => {
    registerMock.mockRejectedValue(new Error("gone"));
    render(<HtmlCanvas artifact={artifact()} />);

    expect(await screen.findByText("This document could not be opened.")).toBeTruthy();
    expect(screen.queryByTitle("Dashboard")).toBeNull();
  });

  it("rejects an empty registration id instead of loading the preview root", async () => {
    registerMock.mockResolvedValue("");
    render(<HtmlCanvas artifact={artifact()} />);

    expect(await screen.findByText("This document could not be opened.")).toBeTruthy();
    expect(registerMock).toHaveBeenCalledTimes(2);
    expect(screen.queryByTitle("Dashboard")).toBeNull();
  });

  it("tries again on request", async () => {
    const user = userEvent.setup();
    registerMock.mockRejectedValue(new Error("gone"));
    render(<HtmlCanvas artifact={artifact()} />);
    await screen.findByText("This document could not be opened.");

    registerMock.mockResolvedValue("recovered");
    await user.click(screen.getByRole("button", { name: "Try again" }));

    expect(await screen.findByTitle("Dashboard")).toBeTruthy();
  });
});

describe("how big the document is", () => {
  it("fills the panel, and offers no device simulation to do it instead", async () => {
    registerMock.mockResolvedValue("fresh-id");
    render(<HtmlCanvas artifact={artifact()} />);

    const frame = await screen.findByTitle("Dashboard");
    expect(frame.className).toContain("w-full");
    expect(frame.className).toContain("h-full");
    // The Desktop/Tablet/Mobile picker is gone: panel width is the size
    // control, and the panel header's expand toggle is how it grows.
    expect(screen.queryByRole("radio", { name: /desktop|tablet|mobile/i })).toBeNull();
  });
});

describe("handing the document over", () => {
  it("copies the source the model wrote", async () => {
    const user = userEvent.setup();
    registerMock.mockResolvedValue("fresh-id");
    render(<HtmlCanvas artifact={artifact()} />);
    await screen.findByTitle("Dashboard");

    await user.click(screen.getByRole("button", { name: "Copy HTML source" }));

    await waitFor(() => expect(clipboard).toHaveBeenCalledWith(artifact().content));
    expect(getToasts()[0]?.text).toBe("Source copied to clipboard.");
  });

  it("saves an html document as html, through the native dialog", async () => {
    const user = userEvent.setup();
    registerMock.mockResolvedValue("fresh-id");
    saveMock.mockResolvedValue({ version: 1, saved: true, file_name: "Dashboard.html" });
    render(<HtmlCanvas artifact={artifact()} />);
    await screen.findByTitle("Dashboard");

    await user.click(screen.getByRole("button", { name: /save file/i }));

    await waitFor(() => expect(saveMock).toHaveBeenCalledTimes(1));
    expect(saveMock.mock.calls[0]).toEqual(["Dashboard", artifact().content, "html"]);
    expect(getToasts()[0]?.text).toBe("Saved Dashboard.html.");
  });

  it("saves an svg artifact under its own format", async () => {
    const user = userEvent.setup();
    registerMock.mockResolvedValue("svg-id");
    saveMock.mockResolvedValue({ version: 1, saved: true, file_name: "Dashboard.svg" });
    render(<HtmlCanvas artifact={artifact({ kind: "svg", content: "<svg></svg>" })} />);
    await screen.findByTitle("Dashboard");

    await user.click(screen.getByRole("button", { name: /save file/i }));

    await waitFor(() => expect(saveMock).toHaveBeenCalled());
    expect(saveMock.mock.calls[0]?.[2]).toBe("svg");
  });

  it("says nothing when the save dialog is dismissed", async () => {
    const user = userEvent.setup();
    registerMock.mockResolvedValue("fresh-id");
    saveMock.mockResolvedValue({ version: 1, saved: false, file_name: null });
    render(<HtmlCanvas artifact={artifact()} />);
    await screen.findByTitle("Dashboard");

    await user.click(screen.getByRole("button", { name: /save file/i }));

    await waitFor(() => expect(saveMock).toHaveBeenCalled());
    expect(getToasts()).toHaveLength(0);
  });
});
