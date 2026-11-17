import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProjectFileStat, ProjectTableRows } from "../../lib/generated/contracts";
import { loadProjectFile, readWindow, type TextWindow } from "./projectFileLoad";
import { ProjectFileViewer, type UnsavedDraftStore } from "./ProjectFileViewer";
import { CanvasPanel } from "../canvas/CanvasPanel";
import { getCanvasState, resetCanvasStore } from "../canvas/canvasStore";
import { Tabs } from "../../ui/Tabs";

// The real Tabs, counted: the viewer renders it on every one of its own renders.
vi.mock("../../ui/Tabs", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ui/Tabs")>();
  return { ...actual, Tabs: vi.fn(actual.Tabs) };
});

const projectFileStatMock = vi.hoisted(() => vi.fn());
const projectDocumentReadMock = vi.hoisted(() => vi.fn());
const projectOfficePreviewMock = vi.hoisted(() => vi.fn());
const projectOfficeSavePdfMock = vi.hoisted(() => vi.fn());
const projectOfficeSaveOriginalMock = vi.hoisted(() => vi.fn());
const projectFileReadBytesMock = vi.hoisted(() => vi.fn());
const projectSaveFileMock = vi.hoisted(() => vi.fn());
const projectFileRevisionsMock = vi.hoisted(() => vi.fn());
const projectFileRestoreMock = vi.hoisted(() => vi.fn());
const projectFileRevisionDiffMock = vi.hoisted(() => vi.fn());
const projectPreviewRegisterMock = vi.hoisted(() => vi.fn(async () => "cafe".repeat(16)));
const previewDocumentRegisterMock = vi.hoisted(() => vi.fn(async () => "beef".repeat(16)));
const projectTableRowsMock = vi.hoisted(() => vi.fn());
const projectSchemaSummaryMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  PROJECT_READ_RANGE_MAX_BYTES: 8 * 1024 * 1024,
  projectFileStat: projectFileStatMock,
  projectDocumentRead: projectDocumentReadMock,
  projectOfficePreview: projectOfficePreviewMock,
  projectOfficeSavePdf: projectOfficeSavePdfMock,
  projectOfficeSaveOriginal: projectOfficeSaveOriginalMock,
  projectFileReadBytes: projectFileReadBytesMock,
  projectSaveFile: projectSaveFileMock,
  projectFileRevisions: projectFileRevisionsMock,
  projectFileRestore: projectFileRestoreMock,
  projectFileRevisionDiff: projectFileRevisionDiffMock,
  projectPreviewRegister: projectPreviewRegisterMock,
  previewDocumentRegister: previewDocumentRegisterMock,
  previewDocumentSrc: (id: string) => `http://xf-preview.localhost/${id}`,
  projectTableRows: projectTableRowsMock,
  projectSchemaSummary: projectSchemaSummaryMock,
  documentExportSave: vi.fn(),
}));

vi.mock("./OfficePdfViewer", () => ({
  OfficePdfViewer: ({ data, name }: { data: Uint8Array; name: string }) => (
    <div data-testid="office-pdf-preview">
      {name}:{data[0]}
    </div>
  ),
}));

/** What a finished preview returns: the PDF bytes and the id that names them for saving. */
const PDF = (renderId: string, first = 37) => ({ data: new Uint8Array([first, 80, 68, 70]), renderId });

const MIB = 1024 * 1024;
/** The line every generated large file repeats: 20 bytes with its newline. */
const LARGE_LINE = new TextEncoder().encode("window line of text\n");

/**
 * A project folder the mocks answer from: `stat` and range reads see the same
 * bytes, so the view's real loader — stat, then ranges, then decode — runs.
 */
type DiskEntry = { bytes: Uint8Array; modified: number } | { stat: ProjectFileStat };
let disk: Map<string, DiskEntry>;

function put(path: string, content: string | Uint8Array, modified = 1_000) {
  disk.set(path, { bytes: typeof content === "string" ? new TextEncoder().encode(content) : content, modified });
}

/** A text file of `size` bytes whose content is never materialized. */
function putLarge(path: string, size: number) {
  disk.set(path, {
    stat: { kind: "text", version: 1, path, size_bytes: size, modified_ms: 1_000 },
  });
}

function statOf(path: string): ProjectFileStat {
  const entry = disk.get(path);
  if (entry === undefined) throw "the resolved path does not exist";
  if ("stat" in entry) return entry.stat;
  return { kind: "text", version: 1, path, size_bytes: entry.bytes.length, modified_ms: entry.modified };
}

beforeEach(() => {
  projectSchemaSummaryMock.mockReset();
  resetCanvasStore();
  disk = new Map();
  projectFileStatMock.mockImplementation(async (_project: string, path: string) => statOf(path));
  projectDocumentReadMock.mockImplementation(async (_project: string, path: string) => {
    const stat = statOf(path);
    if (stat.kind !== "text" && stat.kind !== "not_text") throw new Error("document is withheld");
    return {
      version: 1,
      path,
      text: "Office text extraction\nRead-only document body",
      size_bytes: stat.size_bytes,
      modified_ms: stat.modified_ms,
    };
  });
  projectOfficePreviewMock.mockImplementation(async () => PDF("render-1"));
  projectFileReadBytesMock.mockImplementation(
    async (_project: string, path: string, offset: number, length: number) => {
      const entry = disk.get(path);
      if (entry === undefined) throw "the resolved path does not exist";
      if ("stat" in entry) {
        // A large file is one line repeated end to end, generated for the
        // range asked: byte `p` is the line's byte `p % 20`, wherever a range
        // starts, so windows see the same line breaks the file has.
        const available = Math.max(0, Math.min(length, (entry.stat as { size_bytes: number }).size_bytes - offset));
        const bytes = new Uint8Array(available);
        for (let index = 0; index < available; index += 1) {
          bytes[index] = LARGE_LINE[(offset + index) % LARGE_LINE.length]!;
        }
        return bytes;
      }
      return entry.bytes.slice(offset, offset + length);
    },
  );
  projectSaveFileMock.mockImplementation(async (_project: string, path: string, contentBase64: string) => {
    const binary = atob(contentBase64);
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
    put(path, bytes, 2_000);
  });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function renderViewer(path = "notes.txt") {
  return render(<ProjectFileViewer projectId="project-1" path={path} />);
}

/** Selects the Preview view. It is the default, and showing it never makes the PDF again. */
async function showPreview(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole("radio", { name: "Preview" }));
}

const REVISIONS = {
  version: 1 as const,
  project_id: "project-1",
  relative_path: "notes.txt",
  revisions: [
    {
      version: 1 as const,
      backup_id: "backup-1",
      created_at_ms: 1_700_000_000_000,
      original_bytes: 24,
      compressed_bytes: 18,
      content_hash: "abcdef1234567890",
    },
  ],
};

describe("ProjectFileViewer", () => {
  it.each(["brief.doc", "brief.docx", "slides.ppt", "slides.PPTX"])(
    "routes %s to visual preview and separately exposes extracted text",
    async (path) => {
      put(path, new Uint8Array([0, 1, 2, 3]));
      const user = userEvent.setup();
      const writeText = vi.fn(async () => undefined);
      Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
      renderViewer(path);

      // Preview is the default view, and the PDF is made as the file opens.
      expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent(`${path}:37`);
      expect(projectOfficePreviewMock).toHaveBeenCalledTimes(1);
      expect(screen.queryByRole("button", { name: "Edit" })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Save PDF" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Save a copy" })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Copy text" })).not.toBeInTheDocument();
      expect(projectOfficePreviewMock).toHaveBeenCalledWith("project-1", path, expect.any(AbortSignal));
      await user.click(screen.getByRole("radio", { name: "Text" }));
      expect(await screen.findByText("Office text extraction")).toBeInTheDocument();
      expect(screen.getByText("Read-only document body")).toBeInTheDocument();
      // Going back to the preview shows the PDF already made; it is not made again.
      await user.click(screen.getByRole("radio", { name: "Preview" }));
      expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent(`${path}:37`);
      expect(projectOfficePreviewMock).toHaveBeenCalledTimes(1);
      await user.click(screen.getByRole("radio", { name: "Text" }));
      await user.click(screen.getByRole("button", { name: "Copy text" }));
      expect(writeText).toHaveBeenCalledWith("Office text extraction\nRead-only document body");
      expect(await screen.findByRole("status")).toHaveTextContent("Text copied.");

      await user.click(screen.getByRole("button", { name: "Refresh" }));
      await waitFor(() => expect(projectDocumentReadMock).toHaveBeenCalledTimes(2));
    },
  );

  it("shows valid Office files with no extractable text as read-only", async () => {
    const path = "empty.docx";
    put(path, new Uint8Array([0, 1, 2, 3]));
    projectDocumentReadMock.mockResolvedValueOnce({
      version: 1,
      path,
      text: "",
      size_bytes: 4,
      modified_ms: 1_000,
    });
    renderViewer(path);

    const user = userEvent.setup();
    await showPreview(user);
    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent(`${path}:37`);
    await user.click(screen.getByRole("radio", { name: "Text" }));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "No text could be extracted from this Office file. Images are not OCR-scanned.",
    );
    expect(screen.getByRole("button", { name: "Refresh" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Copy text" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit" })).not.toBeInTheDocument();
    expect(document.querySelector(".project-file-lines__row")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(projectDocumentReadMock).toHaveBeenCalledTimes(2));
  });

  it("reports clipboard failure without enabling document edits", async () => {
    put("brief.docx", new Uint8Array([0, 1, 2]));
    const user = userEvent.setup();
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn(async () => Promise.reject(new Error("clipboard unavailable"))) },
    });
    renderViewer("brief.docx");

    await showPreview(user);
    expect(await screen.findByTestId("office-pdf-preview")).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "Text" }));
    await user.click(screen.getByRole("button", { name: "Copy text" }));
    expect(await screen.findByText("Text could not be copied")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit" })).not.toBeInTheDocument();
  });

  it("scopes the Schemas tab to the file selected in the viewer", async () => {
    put("pit_stops.csv", "driver_id,duration\n1,2\n");
    put("circuits.csv", "circuit_id,country\n1,India\n");
    projectSchemaSummaryMock.mockResolvedValue({
      version: 1,
      project_id: "project-1",
      tables: [
        {
          name: "pit_stops",
          relative_path: "pit_stops.csv",
          format: "csv",
          columns: [],
          sample_rows: [],
          row_count_estimate: 1,
        },
        {
          name: "circuits",
          relative_path: "circuits.csv",
          format: "csv",
          columns: [],
          sample_rows: [],
          row_count_estimate: 1,
        },
      ],
    });
    const user = userEvent.setup();
    const view = renderViewer("pit_stops.csv");
    await user.click(screen.getByRole("tab", { name: "Schemas" }));
    expect(await screen.findByRole("region", { name: "pit_stops" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "circuits" })).not.toBeInTheDocument();

    view.rerender(<ProjectFileViewer projectId="project-1" path="circuits.csv" />);
    await user.click(screen.getByRole("tab", { name: "Schemas" }));
    expect(await screen.findByRole("region", { name: "circuits" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "pit_stops" })).not.toBeInTheDocument();
  });

  it("stats the file, then reads it whole as raw bytes", async () => {
    put("notes.txt", "first line\nsecond line\n");
    renderViewer();

    expect(await screen.findByText("first line")).toBeInTheDocument();
    expect(projectFileStatMock).toHaveBeenCalledWith("project-1", "notes.txt");
    expect(projectFileReadBytesMock).toHaveBeenCalledWith("project-1", "notes.txt", 0, 23);
  });

  it("assembles a file larger than one range from consecutive ranges", async () => {
    const content = "x".repeat(9 * MIB);
    put("big.txt", content);
    renderViewer("big.txt");

    // The whole-file view (and its Edit) appears once both ranges are in.
    await screen.findByRole("button", { name: "Edit" });
    expect(projectFileReadBytesMock.mock.calls.map((call) => [call[2], call[3]])).toEqual([
      [0, 8 * MIB],
      [8 * MIB, MIB],
    ]);
  });

  it("renders only the lines in view, whatever the file's length", async () => {
    put("long.log", Array.from({ length: 50_000 }, (_, index) => `entry ${index}`).join("\n"));
    renderViewer("long.log");

    expect(await screen.findByText("entry 0")).toBeInTheDocument();
    const rows = document.querySelectorAll(".project-file-lines__row");
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.length).toBeLessThan(500);
  });

  it("lists prior revisions only when history is opened", async () => {
    put("notes.txt", "first line\n");
    projectFileRevisionsMock.mockResolvedValueOnce(REVISIONS);
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");

    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));

    expect(await screen.findByRole("button", { name: "Restore revision" })).toBeInTheDocument();
    expect(projectFileRevisionsMock).toHaveBeenCalledWith("project-1", "notes.txt");
  });

  it("loads a revision's changes only when asked, side by side", async () => {
    put("notes.txt", "alpha\ngamma\n");
    projectFileRevisionsMock.mockResolvedValueOnce(REVISIONS);
    projectFileRevisionDiffMock.mockResolvedValueOnce({
      kind: "changes",
      version: 1,
      backup_id: "backup-1",
      unified_diff: "--- a/notes.txt\n+++ b/notes.txt\n@@ -1,2 +1,2 @@\n alpha\n-beta\n+gamma\n",
      added_lines: 1,
      removed_lines: 1,
    });
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("alpha");

    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));
    const show = await screen.findByRole("button", { name: "Show changes" });
    expect(projectFileRevisionDiffMock).not.toHaveBeenCalled();

    await user.click(show);

    expect(projectFileRevisionDiffMock).toHaveBeenCalledWith("project-1", "notes.txt", "backup-1", null);
    const region = await screen.findByRole("region", { name: "Changes since this revision" });
    expect(await within(region).findByTestId("split-diff")).toBeInTheDocument();
    expect(region).toHaveTextContent("1 added · 1 removed. Left: this revision. Right: the file now.");
    // The newest revision has nothing newer to compare with.
    expect(within(region).queryByRole("radiogroup", { name: "Compare this revision with" })).not.toBeInTheDocument();
    expect(within(region).getByText("beta")).toBeInTheDocument();
    expect(within(region).getByText("gamma")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Hide changes" })).toHaveAttribute("aria-expanded", "true");

    await user.click(screen.getByRole("button", { name: "Hide changes" }));
    expect(screen.queryByRole("region", { name: "Changes since this revision" })).not.toBeInTheDocument();
  });

  it("compares an older revision with the next one to show that one change alone", async () => {
    put("notes.txt", "one\ntwo\nthree\n");
    const newer = { ...REVISIONS.revisions[0], backup_id: "backup-2", created_at_ms: 1_700_000_500_000 };
    projectFileRevisionsMock.mockResolvedValueOnce({ ...REVISIONS, revisions: [newer, REVISIONS.revisions[0]] });
    const changes = (backupId: string, unified: string, added: number) => ({
      kind: "changes",
      version: 1,
      backup_id: backupId,
      unified_diff: unified,
      added_lines: added,
      removed_lines: 0,
    });
    projectFileRevisionDiffMock
      .mockResolvedValueOnce(
        changes("backup-1", "--- a/notes.txt\n+++ b/notes.txt\n@@ -1 +1,3 @@\n one\n+two\n+three\n", 2),
      )
      .mockResolvedValueOnce(changes("backup-1", "--- a/notes.txt\n+++ b/notes.txt\n@@ -1 +1,2 @@\n one\n+two\n", 1));
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("one");

    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));
    const shows = await screen.findAllByRole("button", { name: "Show changes" });
    await user.click(shows[1]);
    const region = await screen.findByRole("region", { name: "Changes since this revision" });
    await waitFor(() => expect(region).toHaveTextContent("2 added · 0 removed"));
    expect(projectFileRevisionDiffMock).toHaveBeenLastCalledWith("project-1", "notes.txt", "backup-1", null);

    await user.click(within(region).getByRole("radio", { name: "This change only" }));

    await waitFor(() => expect(region).toHaveTextContent("1 added · 0 removed"));
    expect(projectFileRevisionDiffMock).toHaveBeenLastCalledWith("project-1", "notes.txt", "backup-1", "backup-2");
    expect(region).toHaveTextContent("Right: the ");
    expect(within(region).queryByText("three")).not.toBeInTheDocument();
  });

  it.each([
    [{ kind: "too_large", limit: "file_size" }, "Diff too large to show", "too large to compare here"],
    [{ kind: "too_large", limit: "change_size" }, "Diff too large to show", "changed too much to show as a diff"],
    [{ kind: "not_text" }, "No line diff for this file", "not plain text"],
    [{ kind: "unchanged" }, null, "The file is the same as this revision."],
    [{ kind: "sensitive_withheld", sensitive_class: "dot_env" }, "Changes withheld", "environment file"],
  ])("says why a revision's changes are not shown: %o", async (outcome, title, text) => {
    put("notes.txt", "first line\n");
    projectFileRevisionsMock.mockResolvedValueOnce(REVISIONS);
    projectFileRevisionDiffMock.mockResolvedValueOnce({ version: 1, backup_id: "backup-1", ...outcome });
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");

    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));
    await user.click(await screen.findByRole("button", { name: "Show changes" }));

    const region = await screen.findByRole("region", { name: "Changes since this revision" });
    await waitFor(() => expect(region).toHaveTextContent(text));
    if (title) expect(region).toHaveTextContent(title);
    expect(within(region).queryByTestId("split-diff")).not.toBeInTheDocument();
    // Restoring stays available whatever the diff could show.
    expect(screen.getByRole("button", { name: "Restore revision" })).toBeEnabled();
  });

  it("offers a retry when the diff fails, and refuses a reply for another revision", async () => {
    put("notes.txt", "first line\n");
    projectFileRevisionsMock.mockResolvedValueOnce(REVISIONS);
    projectFileRevisionDiffMock
      .mockResolvedValueOnce({ kind: "unchanged", version: 1, backup_id: "backup-other" })
      .mockResolvedValueOnce({ kind: "unchanged", version: 1, backup_id: "backup-1" });
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");

    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));
    await user.click(await screen.findByRole("button", { name: "Show changes" }));

    expect(await screen.findByText("The diff returned invalid data. Try again.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("The file is the same as this revision.")).toBeInTheDocument();
    expect(projectFileRevisionDiffMock).toHaveBeenCalledTimes(2);
  });

  it("keeps the history loading state until an empty result arrives", async () => {
    put("notes.txt", "first line\n");
    let resolveHistory: (result: typeof REVISIONS | { revisions: [] }) => void = () => undefined;
    projectFileRevisionsMock.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveHistory = resolve;
      }),
    );
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");

    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));
    expect(screen.getByText("Loading revisions…")).toBeInTheDocument();
    await act(async () => resolveHistory({ ...REVISIONS, revisions: [] }));

    expect(await screen.findByText("No revisions yet")).toBeInTheDocument();
    expect(screen.queryByText("Loading revisions…")).not.toBeInTheDocument();
  });

  it("keeps a history failure visible and retries it", async () => {
    put("notes.txt", "first line\n");
    projectFileRevisionsMock
      .mockRejectedValueOnce(new Error("revision store is locked"))
      .mockResolvedValueOnce({ ...REVISIONS, revisions: [] });
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");
    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Revision history could not be loaded. Revision store is locked.",
    );
    await user.click(screen.getByRole("button", { name: "Try again" }));

    expect(await screen.findByText("No revisions yet")).toBeInTheDocument();
    expect(projectFileRevisionsMock).toHaveBeenCalledTimes(2);
  });

  it("keeps the loaded revision list on screen when a manual reload fails", async () => {
    put("notes.txt", "first line\n");
    projectFileRevisionsMock
      .mockResolvedValueOnce(REVISIONS)
      .mockRejectedValueOnce(new Error("revision store is locked"));
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");
    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));
    expect(await screen.findByRole("button", { name: "Restore revision" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Reload history" }));

    // The read history a person already has is not a draft to discard on a
    // hunch that the reload might work.
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Revision history could not be loaded. Revision store is locked.",
    );
    expect(screen.getByRole("button", { name: "Restore revision" })).toBeInTheDocument();
  });

  it("rejects revision history returned for another file", async () => {
    put("notes.txt", "first line\n");
    projectFileRevisionsMock.mockResolvedValueOnce({ ...REVISIONS, relative_path: "other.txt", revisions: [] });
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");
    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Revision history returned invalid data. Try again.");
    expect(screen.queryByRole("button", { name: "Restore revision" })).not.toBeInTheDocument();
  });

  it("restores after one explicit confirmation and reloads the file", async () => {
    put("notes.txt", "first line\n");
    projectFileRevisionsMock.mockResolvedValue(REVISIONS);
    projectFileRestoreMock.mockResolvedValue({ version: 1, project_id: "project-1", relative_path: "notes.txt" });
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");
    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));
    await user.click(await screen.findByRole("button", { name: "Restore revision" }));

    await user.click(screen.getByRole("button", { name: "Restore this revision" }));

    await waitFor(() => expect(projectFileRestoreMock).toHaveBeenCalledWith("backup-1"));
    // Two stats bracket each whole read: the first load's, then the reload's.
    await waitFor(() => expect(projectFileStatMock).toHaveBeenCalledTimes(4));
    expect(screen.getByRole("tab", { name: "File" })).toHaveAttribute("aria-selected", "true");
  });

  it("names the reason a binary file's contents are not shown, and reads none of it", async () => {
    disk.set("logo.png", {
      stat: { kind: "not_text", version: 1, path: "logo.png", size_bytes: 2048, modified_ms: 1, reason: "binary" },
    });
    renderViewer("logo.png");

    expect(await screen.findByText("Contents not shown")).toBeInTheDocument();
    expect(screen.getByText("This file is not text, so there is nothing to show as text.")).toBeInTheDocument();
    expect(screen.getByText("2.0 KB")).toBeInTheDocument();
    expect(projectFileReadBytesMock).not.toHaveBeenCalled();
  });

  it("withholds text that turns out not to be UTF-8 past the sniffed prefix", async () => {
    put("mixed.txt", new Uint8Array([0x68, 0x69, 0xff, 0xfe]));
    renderViewer("mixed.txt");

    expect(
      await screen.findByText("This file is text, but not valid UTF-8, so it cannot be shown without corrupting it."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit" })).not.toBeInTheDocument();
  });

  it("says a sensitive file was withheld, naming its class, and reads nothing", async () => {
    disk.set(".env", {
      stat: { kind: "sensitive_withheld", version: 1, path: ".env", sensitive_class: "dot_env" },
    });
    renderViewer(".env");

    expect(await screen.findByText("This file was not read")).toBeInTheDocument();
    expect(screen.getByText(/environment file/)).toBeInTheDocument();
    expect(projectFileReadBytesMock).not.toHaveBeenCalled();
  });

  it("states the size of a file past the cap and reads nothing", async () => {
    disk.set("dump.sql", {
      stat: {
        kind: "too_large",
        version: 1,
        path: "dump.sql",
        size_bytes: 12 * 1024 * MIB,
        limit_bytes: 10 * 1024 * MIB,
      },
    });
    renderViewer("dump.sql");

    expect(await screen.findByText("Too large to open here")).toBeInTheDocument();
    expect(screen.getByText(/12 GB, larger than the 10 GB this view opens/)).toBeInTheDocument();
    expect(projectFileReadBytesMock).not.toHaveBeenCalled();
  });

  it("writes the file on save, shows what was written, and re-reads nothing", async () => {
    put("notes.txt", "first line\n");
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");
    const reads = projectFileReadBytesMock.mock.calls.length;

    await user.click(screen.getByRole("button", { name: "Edit" }));
    const editor = screen.getByRole("textbox", { name: "File contents" });
    await user.clear(editor);
    await user.type(editor, "replaced");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Saved notes.txt")).toBeInTheDocument();
    expect(projectSaveFileMock).toHaveBeenCalledWith("project-1", "notes.txt", btoa("replaced"));
    expect(screen.getByText("replaced")).toBeInTheDocument();
    expect(projectFileReadBytesMock.mock.calls.length).toBe(reads);
  });

  it("saves a Windows file with its Windows line endings", async () => {
    put("notes.txt", "first\r\nsecond\r\n");
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("second");

    await user.click(screen.getByRole("button", { name: "Edit" }));
    const editor = screen.getByRole("textbox", { name: "File contents" });
    await user.type(editor, "{Control>}{End}{/Control}third");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("Saved notes.txt")).toBeInTheDocument();
    // A text area hands back bare line feeds. The file keeps the endings it had.
    expect(projectSaveFileMock).toHaveBeenCalledWith("project-1", "notes.txt", btoa("first\r\nsecond\r\nthird"));
  });

  it("does not call a file changed when an edit is typed and then undone", async () => {
    put("notes.txt", "first\r\nsecond\r\n");
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("second");

    await user.click(screen.getByRole("button", { name: "Edit" }));
    const editor = screen.getByRole("textbox", { name: "File contents" });
    await user.type(editor, "{Control>}{End}{/Control}x");
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    await user.type(editor, "{Backspace}");

    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("puts unsaved edits back when the file is opened again", async () => {
    put("notes.txt", "first line\n");
    const drafts: UnsavedDraftStore = new Map();
    const user = userEvent.setup();
    const first = render(<ProjectFileViewer drafts={drafts} path="notes.txt" projectId="project-1" />);
    await screen.findByText("first line");
    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.type(screen.getByRole("textbox", { name: "File contents" }), "typed but never saved");

    // The person opens a different file; the editor goes away.
    first.unmount();
    put("other.txt", "elsewhere\n");
    const other = render(<ProjectFileViewer drafts={drafts} path="other.txt" projectId="project-1" />);
    await screen.findByText("elsewhere");
    other.unmount();

    render(<ProjectFileViewer drafts={drafts} path="notes.txt" projectId="project-1" />);
    const editor = await screen.findByRole("textbox", { name: "File contents" });
    expect((editor as HTMLTextAreaElement).value).toContain("typed but never saved");
    expect(screen.getByText("Your unsaved changes are back")).toBeInTheDocument();
  });

  it("does not put an old draft onto a file that changed in the meantime", async () => {
    put("notes.txt", "first line\n");
    const drafts: UnsavedDraftStore = new Map();
    const user = userEvent.setup();
    const first = render(<ProjectFileViewer drafts={drafts} path="notes.txt" projectId="project-1" />);
    await screen.findByText("first line");
    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.type(screen.getByRole("textbox", { name: "File contents" }), "stale edit");
    first.unmount();

    // Something else rewrote the file.
    put("notes.txt", "rewritten elsewhere\n", 5_000);
    render(<ProjectFileViewer drafts={drafts} path="notes.txt" projectId="project-1" />);

    expect(await screen.findByText("rewritten elsewhere")).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: "File contents" })).toBeNull();
    expect(drafts.size).toBe(0);
  });

  it("keeps nothing when no draft store was supplied", async () => {
    put("notes.txt", "first line\n");
    const user = userEvent.setup();
    const first = renderViewer();
    await screen.findByText("first line");
    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.type(screen.getByRole("textbox", { name: "File contents" }), "gone");
    first.unmount();

    renderViewer();
    await screen.findByText("first line");
    expect(screen.queryByRole("textbox", { name: "File contents" })).toBeNull();
  });

  it("keeps the draft when the save fails", async () => {
    put("notes.txt", "first line\n");
    projectSaveFileMock.mockRejectedValueOnce("the disk is full");
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");

    await user.click(screen.getByRole("button", { name: "Edit" }));
    const editor = screen.getByRole("textbox", { name: "File contents" });
    await user.type(editor, "more");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("This file was not saved")).toBeInTheDocument();
    expect(screen.getByText("This file could not be saved. The disk is full.")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "File contents" })).toHaveValue("first line\nmore");
  });

  it("does not overwrite an external edit made without a focus change", async () => {
    put("notes.txt", "first line\n");
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");
    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.type(screen.getByRole("textbox", { name: "File contents" }), "mine");

    // Another program writes the file; its size and time move.
    put("notes.txt", "their edit\n", 5_000);
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("This file changed on disk")).toBeInTheDocument();
    expect(projectSaveFileMock).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("will not save a change that changes nothing", async () => {
    put("notes.txt", "first line\n");
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");
    await user.click(screen.getByRole("button", { name: "Edit" }));
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("notices a file changed on disk by its stat, without swapping the content", async () => {
    put("notes.txt", "first line\n");
    renderViewer();
    await screen.findByText("first line");

    put("notes.txt", "rewritten\n", 9_000);
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });

    expect(await screen.findByText("This file changed on disk")).toBeInTheDocument();
    expect(screen.getByText("first line")).toBeInTheDocument();
    // Detected from size and time: no byte of the file was re-read.
    expect(projectFileReadBytesMock).toHaveBeenCalledTimes(1);
  });

  it("stays quiet when the file's stat has not moved", async () => {
    put("notes.txt", "first line\n");
    renderViewer();
    await screen.findByText("first line");

    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    // The load's two stats, then the focus probe's one.
    await waitFor(() => expect(projectFileStatMock).toHaveBeenCalledTimes(3));
    expect(screen.queryByText("This file changed on disk")).not.toBeInTheDocument();
  });

  it("confirms before a stale-file reload discards an unsaved draft", async () => {
    put("notes.txt", "first line\n");
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");
    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.type(screen.getByRole("textbox", { name: "File contents" }), "draft");

    put("notes.txt", "newer\n", 9_000);
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await screen.findByText("This file changed on disk");
    // The draft is untouched until the person decides.
    expect(screen.getByRole("textbox", { name: "File contents" })).toHaveValue("first line\ndraft");

    await user.click(screen.getByRole("button", { name: "Reload file" }));
    const dialog = screen.getByRole("alertdialog", { name: "Discard unsaved changes?" });
    await user.click(within(dialog).getByRole("button", { name: "Discard and reload" }));

    expect(await screen.findByText("newer")).toBeInTheDocument();
  });

  it("reports a read failure with the backend's own redacted reason", async () => {
    projectFileStatMock.mockRejectedValueOnce("relative path escapes the project root");
    renderViewer();

    expect(await screen.findByText("This file could not be read")).toBeInTheDocument();
    expect(
      screen.getByText("This file could not be read. Relative path escapes the project root."),
    ).toBeInTheDocument();
  });

  it("re-reads the file on demand so a change made elsewhere becomes visible", async () => {
    put("notes.txt", "first line\n");
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");

    put("notes.txt", "approved change\n", 3_000);
    await user.click(screen.getByRole("button", { name: "Reload file" }));

    expect(await screen.findByText("approved change")).toBeInTheDocument();
  });

  it("does not apply a load that finishes after another file was opened", async () => {
    put("notes.txt", "first line\n");
    put("other.txt", "other file\n");
    let release: () => void = () => undefined;
    projectFileStatMock.mockImplementationOnce(
      (_project: string, path: string) =>
        new Promise((resolve) => {
          release = () => resolve(statOf(path));
        }),
    );
    const { rerender } = renderViewer();
    rerender(<ProjectFileViewer projectId="project-1" path="other.txt" />);
    expect(await screen.findByText("other file")).toBeInTheDocument();

    await act(async () => release());
    expect(screen.queryByText("first line")).not.toBeInTheDocument();
  });

  it("shows a restore failure inside its confirmation and allows a retry", async () => {
    put("notes.txt", "first line\n");
    projectFileRevisionsMock.mockResolvedValue(REVISIONS);
    projectFileRestoreMock
      .mockRejectedValueOnce(new Error("backup is unavailable"))
      .mockResolvedValueOnce({ version: 1, project_id: "project-1", relative_path: "notes.txt" });
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");
    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));
    await user.click(await screen.findByRole("button", { name: "Restore revision" }));

    await user.click(screen.getByRole("button", { name: "Restore this revision" }));
    const dialog = screen.getByRole("alertdialog", { name: "Restore this file?" });
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "This revision could not be restored. Backup is unavailable.",
    );

    await user.click(within(dialog).getByRole("button", { name: "Restore this revision" }));
    await waitFor(() => expect(projectFileRestoreMock).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.getByRole("tab", { name: "File" })).toHaveAttribute("aria-selected", "true"));
  });

  it("rejects a restore response naming a different file and keeps the confirmation open", async () => {
    put("notes.txt", "first line\n");
    projectFileRevisionsMock.mockResolvedValue(REVISIONS);
    projectFileRestoreMock.mockResolvedValueOnce({ version: 1, project_id: "project-1", relative_path: "other.txt" });
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");
    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));
    await user.click(await screen.findByRole("button", { name: "Restore revision" }));
    await user.click(screen.getByRole("button", { name: "Restore this revision" }));

    const dialog = screen.getByRole("alertdialog", { name: "Restore this file?" });
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "The restored revision did not match this file. Reload history and try again.",
    );
    expect(within(dialog).getByRole("button", { name: "Restore this revision" })).toBeEnabled();
  });
});

describe("ProjectFileViewer on files too large to hold whole", () => {
  it("pages a 5 GiB file a window at a time, read-only, from its start", async () => {
    putLarge("export.log", 5 * 1024 * MIB);
    const user = userEvent.setup();
    renderViewer("export.log");

    expect(await screen.findAllByText("window line of text")).not.toHaveLength(0);
    expect(projectFileReadBytesMock).toHaveBeenCalledTimes(1);
    expect(projectFileReadBytesMock).toHaveBeenLastCalledWith("project-1", "export.log", 0, MIB);
    expect(screen.getByText(/cannot be edited here/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit" })).not.toBeInTheDocument();
    // Only the window's lines are in the DOM, and only some of those.
    expect(document.querySelectorAll(".project-file-lines__row").length).toBeLessThan(500);

    await user.click(screen.getByRole("button", { name: "Next page" }));
    await waitFor(() => expect(projectFileReadBytesMock).toHaveBeenCalledTimes(2));
    // The first window ends after its last whole line; the next is read from
    // the byte before that, so it can see that its first line starts there.
    const firstEnd = MIB - (MIB % 20);
    const [, , offset, length] = projectFileReadBytesMock.mock.calls[1]!;
    expect([offset, length]).toEqual([firstEnd - 1, MIB + 1]);
    expect(await screen.findByText(`Bytes ${firstEnd.toLocaleString()}–`, { exact: false })).toBeInTheDocument();
  });

  it("pages forward and back through windows without dropping, repeating, or garbling a line", async () => {
    // Multi-byte characters and CRLF endings, so window edges land inside both.
    const lines = Array.from({ length: 100_000 }, (_, index) => `row ${index} café ✓ 😀`);
    const text = `${lines.join("\r\n")}\r\n`;
    put("multi.log", text);
    const size = new TextEncoder().encode(text).length;
    expect(size).toBeGreaterThan(2 * MIB);

    const windows: TextWindow[] = [];
    let offset = 0;
    while (offset < size) {
      const window = await readWindow("project-1", "multi.log", size, offset);
      expect(window.start).toBe(offset);
      expect(window.replaced).toBe(false);
      windows.push(window);
      offset = window.end;
    }
    expect(windows.length).toBeGreaterThan(1);
    expect(windows.flatMap((window) => window.lines)).toEqual(lines);

    // Back from a window's start ends exactly where that window begins.
    const last = windows[windows.length - 1]!;
    const previous = await readWindow("project-1", "multi.log", size, Math.max(0, last.start - MIB));
    expect(previous.end).toBe(last.start);
  });

  it("cuts a line longer than a window at character boundaries", async () => {
    // One line of three-byte characters: a window's byte edge falls mid-character.
    put("one-line.txt", "✓".repeat(MIB));
    const size = 3 * MIB;

    const first = await readWindow("project-1", "one-line.txt", size, 0);
    const second = await readWindow("project-1", "one-line.txt", size, first.end);
    for (const window of [first, second]) {
      expect(window.replaced).toBe(false);
      expect(window.lines.join("")).not.toContain("\uFFFD");
      expect((window.end - window.start) % 3).toBe(0);
    }
    expect(second.start).toBe(first.end);
  });

  it("pages a large CSV's rows from the backend instead of parsing it here", async () => {
    putLarge("events.csv", 2 * 1024 * MIB);
    const window = (start: number): ProjectTableRows => ({
      version: 1,
      path: "events.csv",
      headers: ["id", "event"],
      rows: Array.from({ length: 500 }, (_, index) => [String(start + index), "open"]),
      start_row: start,
      next_row: start + 500,
      total_rows: null,
      truncated: false,
    });
    projectTableRowsMock.mockImplementation(async (_p: string, _path: string, _d: string, start: number) =>
      window(start),
    );
    const user = userEvent.setup();
    renderViewer("events.csv");

    expect(await screen.findByText("Rows 1–500 of 500+")).toBeInTheDocument();
    expect(projectTableRowsMock).toHaveBeenCalledWith("project-1", "events.csv", ",", 0, 500);
    expect(projectFileReadBytesMock).not.toHaveBeenCalled();
    expect(screen.getByRole("columnheader", { name: "event" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Next page" }));
    expect(await screen.findByText("Rows 501–1,000 of 1,000+")).toBeInTheDocument();

    await user.type(screen.getByRole("textbox", { name: "Go to row" }), "5001");
    await user.click(screen.getByRole("button", { name: "Go" }));
    await waitFor(() =>
      expect(projectTableRowsMock).toHaveBeenLastCalledWith("project-1", "events.csv", ",", 5000, 500),
    );
  });

  it("charts only the current large CSV page and refuses truncated cells", async () => {
    putLarge("events.csv", 2 * 1024 * MIB);
    projectTableRowsMock.mockImplementation(
      async (_p: string, _path: string, _d: string, start: number) =>
        ({
          version: 1,
          path: "events.csv",
          headers: ["id", "event"],
          rows:
            start === 0
              ? [
                  ["1", "open"],
                  ["2", "open"],
                ]
              : [["3", "close"]],
          start_row: start,
          next_row: start === 0 ? 2 : 3,
          total_rows: null,
          truncated: start === 3,
        }) satisfies ProjectTableRows,
    );
    render(
      <>
        <ProjectFileViewer path="events.csv" projectId="project-1" />
        <CanvasPanel />
      </>,
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Chart page in panel" }));
    expect(screen.getByRole("img", { name: "events.csv — rows 1–2, bar chart" })).toBeInTheDocument();
    expect(getCanvasState().tabs[0]).toMatchObject({ data: [{ x: "open", y0: 2 }] });

    await user.click(screen.getByRole("button", { name: "Next page" }));
    await screen.findByText("Rows 3–3 of 3+");
    await user.click(screen.getByRole("button", { name: "Chart page in panel" }));
    expect(screen.getByRole("img", { name: "events.csv — rows 3–3, bar chart" })).toBeInTheDocument();
    expect(getCanvasState().tabs[1]).toMatchObject({ data: [{ x: "close", y0: 1 }] });

    await user.click(screen.getByRole("button", { name: "Next page" }));
    await screen.findByText("Some cells or columns are too long to show whole and are cut here.");
    expect(screen.getByRole("button", { name: "Chart page in panel" })).toBeDisabled();
    expect(getCanvasState().tabs).toHaveLength(2);
    expect(projectFileReadBytesMock).not.toHaveBeenCalled();
  });

  it("refuses a page chart with too many measure values while keeping pagination available", async () => {
    putLarge("metrics.csv", 2 * 1024 * MIB);
    projectTableRowsMock.mockImplementation(
      async (_p: string, _path: string, _d: string, start: number) =>
        ({
          version: 1,
          path: "metrics.csv",
          headers: Array.from({ length: 11 }, (_, index) => `Metric ${index + 1}`),
          rows: Array.from({ length: start === 0 ? 500 : 1 }, () => Array<string>(11).fill("10")),
          start_row: start,
          next_row: start === 0 ? 500 : null,
          total_rows: 501,
          truncated: false,
        }) satisfies ProjectTableRows,
    );
    render(
      <>
        <ProjectFileViewer path="metrics.csv" projectId="project-1" />
        <CanvasPanel />
      </>,
    );
    const user = userEvent.setup();
    expect(await screen.findByText("This table has too many values to chart (maximum 5,000).")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Chart page in panel" })).toBeDisabled();
    expect(getCanvasState().tabs).toHaveLength(0);

    await user.click(screen.getByRole("button", { name: "Next page" }));
    await screen.findByText("Rows 501–501 of 501");
    await user.click(screen.getByRole("button", { name: "Chart page in panel" }));
    expect(
      screen.getByRole("img", { name: "metrics.csv — rows 501–501, bar chart" }).querySelectorAll("rect"),
    ).toHaveLength(11);
    expect(screen.queryByText("This table has too many values to chart (maximum 5,000).")).toBeNull();
  });
});

describe("ProjectFileViewer renders a file the way the chat renders its type", () => {
  const dashboard =
    "<!DOCTYPE html><html><head><title>Q3 revenue</title></head><body><div>Bookings by region for the third quarter, charted</div></body></html>";

  it("previews an HTML file registered from disk, in the chat's sandboxed frame", async () => {
    put("reports/Q3.html", dashboard);
    renderViewer("reports/Q3.html");

    const frame = await screen.findByTitle("Q3 revenue");
    expect(frame.tagName).toBe("IFRAME");
    expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    expect(frame).toHaveAttribute("src", `http://xf-preview.localhost/${"cafe".repeat(16)}`);
    // Registered by path: the renderer never sends the document.
    expect(projectPreviewRegisterMock).toHaveBeenCalledWith("project-1", "reports/Q3.html", "html");
    expect(previewDocumentRegisterMock).not.toHaveBeenCalled();
    const copies = screen.getAllByText(/Bookings by region/);
    expect(copies.map((node) => node.tagName)).toEqual(["CODE"]);
    expect(copies[0]?.closest("pre")).toHaveClass("hidden");
  });

  it("previews an HTML file past the chat's 2 MiB bound, and withholds the side panel", async () => {
    put(
      "reports/big.html",
      `<!doctype html><html><head><title>Big</title></head><body>${"x".repeat(3 * MIB)}</body></html>`,
    );
    renderViewer("reports/big.html");

    expect(await screen.findByTitle("Big")).toHaveAttribute("sandbox", "allow-scripts");
    expect(screen.queryByRole("button", { name: /into the side panel/ })).not.toBeInTheDocument();
  });

  it("previews an SVG file as an image document", async () => {
    const svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10"/></svg>\n';
    put("charts/box.svg", svg);
    renderViewer("charts/box.svg");

    expect(await screen.findByTitle("SVG image")).toHaveAttribute("sandbox", "allow-scripts");
    expect(projectPreviewRegisterMock).toHaveBeenCalledWith("project-1", "charts/box.svg", "svg");
  });

  it("draws a CSV file as the chat's table, cells as text", async () => {
    put("data/regions.csv", "region,amount\nEMEA,10\nAPAC,<b>20</b>\nAMER,30\n");
    renderViewer("data/regions.csv");

    const table = await screen.findByRole("table");
    expect(within(table).getAllByRole("row")).toHaveLength(4);
    expect(screen.getByText("3 rows · 2 columns")).toBeInTheDocument();
    expect(within(table).getByText("<b>20</b>")).toBeInTheDocument();
    expect(table.querySelector("b")).toBeNull();
  });

  it.each([
    ["csv", ","],
    ["tsv", "\t"],
  ])("charts categorical project %s rows without treating IDs as measures", async (extension, delimiter) => {
    const path = `global_geography_trivia.${extension}`;
    const records = [
      ["ID", "Category", "Question", "Answer"],
      ["1", "Rivers", "Longest river?", "Nile"],
      ["2", "Mountains", "Tallest mountain?", "Everest"],
      ["3", "Rivers", "Largest river by flow?", "Amazon"],
    ];
    put(path, records.map((row) => row.join(delimiter)).join("\n"));
    render(
      <>
        <ProjectFileViewer path={path} projectId="project-1" />
        <CanvasPanel />
      </>,
    );

    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: `Chart ${path} in the side panel` }));
    expect(screen.getByRole("img", { name: `${path}, bar chart` }).querySelectorAll("rect")).toHaveLength(2);
    expect(screen.getByText("Row count by Category")).toBeInTheDocument();
    expect(getCanvasState().tabs[0]).toMatchObject({
      data: [
        { x: "Rivers", y0: 2 },
        { x: "Mountains", y0: 1 },
      ],
      seriesLabels: { y0: "Row count by Category" },
    });
  });

  it("charts numeric-only CSV measures against row numbers", async () => {
    put("metrics.csv", "Revenue,Profit\n100,10\n200,-20\n");
    render(
      <>
        <ProjectFileViewer path="metrics.csv" projectId="project-1" />
        <CanvasPanel />
      </>,
    );

    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Chart metrics.csv in the side panel" }));
    expect(screen.getByRole("img", { name: "metrics.csv, bar chart" }).querySelectorAll("rect")).toHaveLength(4);
    expect(getCanvasState().tabs[0]).toMatchObject({
      data: [
        { x: "Row 1", y0: 100, y1: 10 },
        { x: "Row 2", y0: 200, y1: -20 },
      ],
      seriesLabels: { y0: "Revenue", y1: "Profit" },
    });
  });

  it("counts the complete CSV beyond the inline table's visible row cap", async () => {
    const rows = Array.from({ length: 104 }, (_, index) => `${index + 1},${index < 102 ? "Rivers" : "Mountains"}`);
    put("categories.csv", `ID,Category\n${rows.join("\n")}\n`);
    renderViewer("categories.csv");
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Chart categories.csv in the side panel" }));
    expect(getCanvasState().tabs[0]).toMatchObject({
      data: [
        { x: "Rivers", y0: 102 },
        { x: "Mountains", y0: 2 },
      ],
    });
  });

  it.each([
    ["Category", Array.from({ length: 100_000 }, (_, index) => `category-${index}`)],
    ["Revenue,Profit", Array.from({ length: 2_501 }, () => "100,10")],
  ])("refuses an oversized CSV chart for %s without dropping table rows", async (headers, rows) => {
    put("many.csv", `${headers}\n${rows.join("\n")}\n`);
    render(
      <>
        <ProjectFileViewer path="many.csv" projectId="project-1" />
        <CanvasPanel />
      </>,
    );
    expect(await screen.findByText("This table has too many values to chart (maximum 5,000).")).toBeInTheDocument();
    expect(
      screen.getByText(
        `${rows.length.toLocaleString()} rows · ${headers.split(",").length} column${headers.includes(",") ? "s" : ""}`,
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Chart many.csv in the side panel" })).toBeNull();
    expect(screen.getByRole("button", { name: "Open many.csv in the side panel" })).toBeEnabled();
    expect(getCanvasState().tabs).toHaveLength(0);
  });

  it("still charts every row of a large CSV when its category counts fit the chart limit", async () => {
    const rows = Array.from({ length: 100_000 }, (_, index) => (index % 2 === 0 ? "Rivers" : "Mountains"));
    put("counts.csv", `Category\n${rows.join("\n")}\n`);
    render(
      <>
        <ProjectFileViewer path="counts.csv" projectId="project-1" />
        <CanvasPanel />
      </>,
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Chart counts.csv in the side panel" }));
    expect(screen.getByRole("img", { name: "counts.csv, bar chart" }).querySelectorAll("rect")).toHaveLength(2);
    expect(getCanvasState().tabs[0]).toMatchObject({
      data: [
        { x: "Rivers", y0: 50_000 },
        { x: "Mountains", y0: 50_000 },
      ],
    });
  });

  it("charts repeated identifiers as row counts without using ID values as measures", async () => {
    put("ids.csv", "id\n42\n42\n99\n");
    render(
      <>
        <ProjectFileViewer path="ids.csv" projectId="project-1" />
        <CanvasPanel />
      </>,
    );
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Chart ids.csv in the side panel" }));
    expect(screen.getByRole("img", { name: "ids.csv, bar chart" }).querySelectorAll("rect")).toHaveLength(2);
    expect(getCanvasState().tabs[0]).toMatchObject({
      data: [
        { x: "42", y0: 2 },
        { x: "99", y0: 1 },
      ],
      seriesLabels: { y0: "Row count by id" },
    });
  });

  it("tables a JSON array of objects and highlights any other JSON", async () => {
    put("rows.json", '[{"id":1,"name":"a"},{"id":2,"name":"b"}]');
    renderViewer("rows.json");
    expect(within(await screen.findByRole("table")).getAllByRole("row")).toHaveLength(3);
    cleanup();

    put("config.json", '{"retries": 3}');
    renderViewer("config.json");
    const card = await screen.findByRole("region", { name: "JSON" });
    expect(card.querySelector(".hljs-attr")).toHaveTextContent('"retries"');
  });

  it("tables a JSON file that starts with a byte-order mark", async () => {
    put("bom.json", '\uFEFF[{"id":1},{"id":2}]');
    renderViewer("bom.json");

    const table = await screen.findByRole("table");
    expect(within(table).getAllByRole("row")).toHaveLength(3);
    expect(within(table).getByRole("columnheader", { name: "id" })).toBeInTheDocument();
  });

  it("keeps a CRLF file's carriage returns out of its highlighted lines", async () => {
    put("crlf.sql", "SELECT 1\r\n-- note\r\n");
    renderViewer("crlf.sql");

    const card = await screen.findByRole("region", { name: "SQL" });
    expect(card.querySelector(".hljs-comment")?.textContent).toBe("-- note");
    expect(within(card).getByText("2 lines")).toBeInTheDocument();
  });

  it("shows an HTML file past the preview bound as plain lines, saying why", async () => {
    put("reports/huge.html", `<html><body>${"x".repeat(33 * MIB)}</body></html>`);
    renderViewer("reports/huge.html");

    expect(await screen.findByText("This file is too large to preview, so it is shown as text.")).toBeInTheDocument();
    expect(projectPreviewRegisterMock).not.toHaveBeenCalled();
    expect(document.querySelector("iframe")).toBeNull();
  });

  it("tables a JSON Lines file whose every line is an object", async () => {
    put("events.jsonl", '{"event":"open"}\n\n{"event":"close"}\n');
    renderViewer("events.jsonl");

    expect(within(await screen.findByRole("table")).getAllByRole("row")).toHaveLength(3);
  });

  it("renders Markdown through the chat's sanitizer and ladder", async () => {
    put(
      "README.md",
      "# Pipeline notes\n\nSome **bold** text.\n\n<script>window.__pwned = true</script>\n\n```sql\nSELECT 1\n```\n",
    );
    renderViewer("README.md");

    expect(await screen.findByRole("heading", { name: "Pipeline notes" })).toBeInTheDocument();
    expect(screen.getByText("bold").tagName).toBe("STRONG");
    expect(screen.getByRole("region", { name: "SQL" })).toBeInTheDocument();
    expect(document.querySelector("script")).toBeNull();
    expect((window as { __pwned?: boolean }).__pwned).toBeUndefined();
  });

  it("highlights a SQL file as virtualized lines under the registry's name", async () => {
    put("queries/pipeline.snowflake.sql", "SELECT region\nFROM bookings;\n");
    renderViewer("queries/pipeline.snowflake.sql");

    const card = await screen.findByRole("region", { name: "SQL" });
    expect(card.querySelector(".hljs-keyword")).toHaveTextContent("SELECT");
    expect(within(card).getByText("2 lines")).toBeInTheDocument();
  });

  it("keeps a comment's scope across the lines it spans", async () => {
    put("query.sql", "/* first\nsecond */\nSELECT 1;\n");
    renderViewer("query.sql");

    const card = await screen.findByRole("region", { name: "SQL" });
    const comments = [...card.querySelectorAll(".hljs-comment")].map((node) => node.textContent);
    expect(comments).toEqual(["/* first", "second */"]);
  });

  it("shows a code file too large to highlight as plain lines, saying why", async () => {
    put("dump.sql", "SELECT 1;\n".repeat(220_000));
    renderViewer("dump.sql");

    expect(
      await screen.findByText("This file is too large to highlight, so it is shown as plain text."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "SQL" })).not.toBeInTheDocument();
  });

  it("falls back to plain lines for malformed content and unknown types", async () => {
    put("broken.csv", 'a,b\n"unterminated,1\n');
    renderViewer("broken.csv");
    expect(
      await screen.findByText("This file could not be read as a table, so it is shown as text."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    cleanup();

    put("data.xyz", "<b>raw</b>");
    renderViewer("data.xyz");
    const line = await screen.findByText("<b>raw</b>");
    expect(line.closest(".project-file-lines")).not.toBeNull();
    expect(document.querySelector("b")).toBeNull();
  });

  it("edits the raw text and renders the saved text again afterwards", async () => {
    put("README.md", "# Old title\n");
    const user = userEvent.setup();
    renderViewer("README.md");
    expect(await screen.findByRole("heading", { name: "Old title" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Edit" }));
    const editor = screen.getByRole("textbox", { name: "File contents" });
    expect(editor).toHaveValue("# Old title\n");
    await user.clear(editor);
    await user.type(editor, "# New title");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByRole("heading", { name: "New title" })).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: "File contents" })).not.toBeInTheDocument();
  });

  it("does not offer editing for a file past the editor's bound", async () => {
    put("large.txt", "y".repeat(17 * MIB));
    renderViewer("large.txt");

    expect(await screen.findByText(/read-only here/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Edit" })).toBeDisabled();
  });
});

describe("loadProjectFile", () => {
  it("routes supported Office suffixes through read-only extraction and withholds sensitive files", async () => {
    for (const path of ["word.doc", "word.docx", "slides.ppt", "slides.PPTX"]) {
      put(path, new Uint8Array([0, 1, 2]));
      expect(await loadProjectFile("project-1", path)).toMatchObject({
        mode: "document",
        text: "Office text extraction\nRead-only document body",
      });
    }
    disk.set(".env.docx", {
      stat: { kind: "sensitive_withheld", version: 1, path: ".env.docx", sensitive_class: "dot_env" },
    });

    expect(await loadProjectFile("project-1", ".env.docx")).toMatchObject({ mode: "opaque" });
    expect(projectDocumentReadMock).toHaveBeenCalledTimes(4);
  });

  it.each([
    { version: 2, path: "notes.docx", text: "text", size_bytes: 4, modified_ms: 1_000 },
    { version: 1, path: "other.docx", text: "text", size_bytes: 4, modified_ms: 1_000 },
    { version: 1, path: "notes.docx", text: 42, size_bytes: 4, modified_ms: 1_000 },
    { version: 1, path: "notes.docx", text: "text", size_bytes: -1, modified_ms: 1_000 },
    { version: 1, path: "notes.docx", text: "text", size_bytes: 4, modified_ms: Number.NaN },
  ])("rejects malformed Office extraction responses %#", async (response) => {
    put("notes.docx", new Uint8Array([0, 1, 2, 3]));
    projectDocumentReadMock.mockResolvedValueOnce(response);

    await expect(loadProjectFile("project-1", "notes.docx")).rejects.toBeDefined();
  });

  it("does not start Office extraction when the initial stat was aborted", async () => {
    put("notes.docx", new Uint8Array([0, 1, 2]));
    const controller = new AbortController();
    projectFileStatMock.mockImplementationOnce(async (_project: string, path: string) => {
      controller.abort();
      return statOf(path);
    });

    await expect(loadProjectFile("project-1", "notes.docx", undefined, controller.signal)).rejects.toBeDefined();
    expect(projectDocumentReadMock).not.toHaveBeenCalled();
  });

  it("retries Office extraction when the source changes during the read", async () => {
    put("notes.docx", new Uint8Array([0, 1, 2, 3]));
    projectDocumentReadMock.mockImplementationOnce(async (_project: string, path: string) => {
      const stale = statOf(path);
      if (stale.kind !== "text" && stale.kind !== "not_text") throw new Error("fixture stat is unreadable");
      put(path, new Uint8Array([0, 1, 2, 3, 4]), 2_000);
      return {
        version: 1,
        path,
        text: "stale extraction",
        size_bytes: stale.size_bytes,
        modified_ms: stale.modified_ms,
      };
    });

    const loaded = await loadProjectFile("project-1", "notes.docx");

    expect(loaded).toMatchObject({ mode: "document", text: "Office text extraction\nRead-only document body" });
    expect(projectDocumentReadMock).toHaveBeenCalledTimes(2);
  });

  it("reads again when the file changes while it is being read", async () => {
    put("notes.txt", "old content\n");
    projectFileReadBytesMock.mockImplementationOnce(async (_project: string, path: string) => {
      const stale = (disk.get(path) as { bytes: Uint8Array }).bytes.slice();
      put(path, "new content, longer\n", 2_000);
      return stale;
    });

    const loaded = await loadProjectFile("project-1", "notes.txt");

    expect(loaded).toMatchObject({ mode: "whole", text: "new content, longer\n", stat: { modified_ms: 2_000 } });
    expect(projectFileReadBytesMock).toHaveBeenCalledTimes(2);
  });

  it("stops reading once its signal aborts", async () => {
    put("big.txt", "x".repeat(9 * MIB));
    const controller = new AbortController();
    const read = projectFileReadBytesMock.getMockImplementation()!;
    projectFileReadBytesMock.mockImplementationOnce(async (...args: [string, string, number, number]) => {
      controller.abort();
      return read(...args);
    });

    await expect(loadProjectFile("project-1", "big.txt", undefined, controller.signal)).rejects.toBeDefined();
    expect(projectFileReadBytesMock).toHaveBeenCalledTimes(1);
  });
});

describe("a file an agent changes while it is open", () => {
  it("is replaced on screen when nobody is editing it", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      put("notes.txt", "before\n");
      renderViewer();
      await screen.findByText("before");

      put("notes.txt", "after the agent's edit\n", 5_000);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(3_500);
      });

      expect(await screen.findByText("after the agent's edit")).toBeInTheDocument();
      expect(screen.queryByText("This file changed on disk")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("is never swapped under someone who is typing in it", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      put("notes.txt", "before\n");
      const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
      renderViewer();
      await screen.findByText("before");
      await user.click(screen.getByRole("button", { name: "Edit" }));
      await user.type(screen.getByRole("textbox", { name: "File contents" }), "my words");

      put("notes.txt", "after the agent's edit\n", 5_000);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(3_500);
      });

      expect(await screen.findByText("This file changed on disk")).toBeInTheDocument();
      expect((screen.getByRole("textbox", { name: "File contents" }) as HTMLTextAreaElement).value).toContain(
        "my words",
      );
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("ProjectFileViewer across re-renders of its owner", () => {
  it("does not render again when its owner re-renders with the same file", async () => {
    put("notes.txt", "first line\n");
    put("other.txt", "other file\n");
    const { rerender } = renderViewer();
    await screen.findByText("first line");
    const rendersBefore = vi.mocked(Tabs).mock.calls.length;

    rerender(<ProjectFileViewer projectId="project-1" path="notes.txt" />);
    expect(vi.mocked(Tabs).mock.calls.length).toBe(rendersBefore);

    // The same count moves when the file really changes, so it is not stuck.
    rerender(<ProjectFileViewer projectId="project-1" path="other.txt" />);
    expect(vi.mocked(Tabs).mock.calls.length).toBeGreaterThan(rendersBefore);
  });

  it("loads the new file when the path changes", async () => {
    put("notes.txt", "first line\n");
    put("other.txt", "other file\n");
    const { rerender } = renderViewer();
    await screen.findByText("first line");

    rerender(<ProjectFileViewer projectId="project-1" path="other.txt" />);

    expect(await screen.findByText("other file")).toBeInTheDocument();
    expect(screen.queryByText("first line")).toBeNull();
    expect(projectFileStatMock).toHaveBeenCalledWith("project-1", "other.txt");
  });

  it("cancels and hides a superseded Office response after navigation", async () => {
    put("old.docx", new Uint8Array([0, 1]));
    put("new.pptx", new Uint8Array([0, 1, 2]));
    const oldResponse: {
      resolve: ((value: ReturnType<typeof PDF>) => void) | null;
      signal: AbortSignal | null;
    } = {
      resolve: null,
      signal: null,
    };
    projectOfficePreviewMock.mockImplementationOnce(
      (_project: string, _path: string, signal: AbortSignal) =>
        new Promise<ReturnType<typeof PDF>>((resolve) => {
          oldResponse.resolve = resolve;
          oldResponse.signal = signal;
        }),
    );
    const user = userEvent.setup();
    const { rerender } = renderViewer("old.docx");
    await showPreview(user);
    await waitFor(() =>
      expect(projectOfficePreviewMock).toHaveBeenCalledWith("project-1", "old.docx", expect.any(AbortSignal)),
    );

    rerender(<ProjectFileViewer projectId="project-1" path="new.pptx" />);
    await showPreview(user);
    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent("new.pptx:37");
    expect(oldResponse.signal?.aborted).toBe(true);
    oldResponse.resolve?.(PDF("render-old", 1));

    expect(screen.queryByTestId("office-pdf-preview")).toHaveTextContent("new.pptx:37");
    expect(screen.queryByText("stale navigation result")).not.toBeInTheDocument();
  });

  it("cancels native preview work when the project viewer leaves the File tab", async () => {
    put("slides.pptx", new Uint8Array([0, 1, 2]));
    const pending: { signal: AbortSignal | null } = { signal: null };
    projectOfficePreviewMock.mockImplementationOnce((_project: string, _path: string, signal: AbortSignal) => {
      pending.signal = signal;
      return new Promise<ReturnType<typeof PDF>>(() => {});
    });
    const user = userEvent.setup();
    renderViewer("slides.pptx");

    await showPreview(user);
    await waitFor(() => expect(pending.signal).toBeInstanceOf(AbortSignal));
    await user.click(screen.getByRole("tab", { name: "History & Revisions" }));
    expect(pending.signal?.aborted).toBe(true);
    await user.click(screen.getByRole("tab", { name: "File" }));
    await showPreview(user);
    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent("slides.pptx:37");
  });

  it("keeps visual preview available when text extraction fails, and withholds sensitive files before either read", async () => {
    put("broken-text.docx", new Uint8Array([0, 1, 2]));
    projectDocumentReadMock.mockRejectedValueOnce(new Error("text reader unavailable"));
    const user = userEvent.setup();
    renderViewer("broken-text.docx");

    await showPreview(user);
    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent("broken-text.docx:37");
    await user.click(screen.getByRole("radio", { name: "Text" }));
    expect(await screen.findByText(/text reader unavailable/i)).toBeInTheDocument();

    cleanup();
    disk.set(".secrets.docx", {
      stat: { kind: "sensitive_withheld", version: 1, path: ".secrets.docx", sensitive_class: "dot_env" },
    });
    renderViewer(".secrets.docx");
    expect(await screen.findByText(/classified as environment file/i)).toBeInTheDocument();
    expect(projectOfficePreviewMock).toHaveBeenCalledTimes(1);
    expect(projectDocumentReadMock).toHaveBeenCalledTimes(1);
  });

  it("keeps extracted text available when the visual renderer is unavailable and refuses oversized Office sources", async () => {
    put("preview-unavailable.pptx", new Uint8Array([0, 1, 2]));
    projectOfficePreviewMock.mockRejectedValueOnce(new Error("runtime unavailable"));
    const user = userEvent.setup();
    renderViewer("preview-unavailable.pptx");

    expect(await screen.findByText(/runtime unavailable/i)).toBeInTheDocument();
    expect(screen.queryByTestId("office-pdf-preview")).not.toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "Text" }));
    expect(await screen.findByText(/Office text extraction/)).toBeInTheDocument();

    cleanup();
    disk.set("large.docx", {
      stat: { kind: "text", version: 1, path: "large.docx", size_bytes: 40 * MIB, modified_ms: 1_000 },
    });
    renderViewer("large.docx");
    expect(await screen.findByText(/This Office file is 40 MB. Files larger than 32 MB/)).toBeInTheDocument();
    expect(projectOfficePreviewMock).toHaveBeenCalledTimes(1);
  });

  it("shows extracted text while native visual conversion is still pending", async () => {
    put("pending-preview.docx", new Uint8Array([0, 1, 2]));
    let resolvePreview: ((value: ReturnType<typeof PDF>) => void) | null = null;
    projectOfficePreviewMock.mockImplementationOnce(
      () =>
        new Promise<ReturnType<typeof PDF>>((resolve) => {
          resolvePreview = resolve;
        }),
    );
    const user = userEvent.setup();
    renderViewer("pending-preview.docx");

    await showPreview(user);
    expect(await screen.findByText("Making the PDF preview…")).toBeInTheDocument();
    await user.click(await screen.findByRole("radio", { name: "Text" }));
    expect(await screen.findByText("Office text extraction")).toBeInTheDocument();
    expect(resolvePreview).not.toBeNull();
  });

  it("shows a rendered PDF while Office text extraction is still pending", async () => {
    put("pending-text.pptx", new Uint8Array([0, 1, 2]));
    const pendingText: { resolve: ((value: unknown) => void) | null } = { resolve: null };
    projectDocumentReadMock.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          pendingText.resolve = resolve;
        }),
    );
    const user = userEvent.setup();
    renderViewer("pending-text.pptx");

    await showPreview(user);
    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent("pending-text.pptx:37");
    await user.click(screen.getByRole("radio", { name: "Text" }));
    expect(screen.getByText("Reading extracted text…")).toBeInTheDocument();
    pendingText.resolve?.({
      version: 1,
      path: "pending-text.pptx",
      text: "Text arrived after the preview.",
      size_bytes: 3,
      modified_ms: 1_000,
    });
    expect(await screen.findByText("Text arrived after the preview.")).toBeInTheDocument();
  });

  it("retries both Office results when extraction reports a different source version", async () => {
    put("changing.docx", new Uint8Array([0, 1, 2]), 1_000);
    projectDocumentReadMock.mockImplementationOnce(async (_project: string, path: string) => {
      const oldStat = statOf(path);
      if (oldStat.kind !== "text") throw new Error("fixture stat was not text");
      put(path, new Uint8Array([0, 1, 2, 3]), 2_000);
      return {
        version: 1,
        path,
        text: "Stale extracted text.",
        size_bytes: oldStat.size_bytes,
        modified_ms: oldStat.modified_ms,
      };
    });
    projectOfficePreviewMock.mockResolvedValueOnce(PDF("render-old", 1));
    renderViewer("changing.docx");

    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent("changing.docx:37");
    // The old version's PDF is dropped and one is made for the new version.
    expect(projectOfficePreviewMock).toHaveBeenCalledTimes(2);
    expect(screen.queryByText("Stale extracted text.")).not.toBeInTheDocument();
    expect(projectDocumentReadMock).toHaveBeenCalledTimes(3);
    await userEvent.setup().click(screen.getByRole("radio", { name: "Text" }));
    expect(await screen.findByText("Office text extraction")).toBeInTheDocument();
  });

  it("hides both views when a later permission check fails", async () => {
    put("revoked.doc", new Uint8Array([0, 1, 2]));
    renderViewer("revoked.doc");

    const user = userEvent.setup();
    await showPreview(user);
    expect(await screen.findByTestId("office-pdf-preview")).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "Text" }));
    expect(await screen.findByText("Office text extraction")).toBeInTheDocument();
    projectFileStatMock.mockRejectedValueOnce(new Error("permission revoked"));
    act(() => window.dispatchEvent(new Event("focus")));

    expect(
      await screen.findByText(/Access to this Office file is no longer available. Permission revoked./),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("office-pdf-preview")).not.toBeInTheDocument();
    expect(screen.queryByText("Office text extraction")).not.toBeInTheDocument();
  });

  it("previews a spreadsheet the same way, without a text view", async () => {
    put("book.xlsx", new Uint8Array([0, 1, 2]));
    renderViewer("book.xlsx");

    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent("book.xlsx:37");
    expect(screen.queryByRole("radio", { name: "Text" })).not.toBeInTheDocument();
    expect(projectDocumentReadMock).not.toHaveBeenCalled();
    expect(projectOfficePreviewMock).toHaveBeenCalledWith("project-1", "book.xlsx", expect.any(AbortSignal));
  });

  it("saves the PDF and a copy through the native dialog, and says nothing when it is dismissed", async () => {
    put("deck.pptx", new Uint8Array([0, 1, 2]));
    projectOfficeSavePdfMock.mockResolvedValueOnce({ version: 1, saved: true, file_name: "deck.pdf" });
    projectOfficeSaveOriginalMock.mockResolvedValueOnce({ version: 1, saved: false, file_name: null });
    const user = userEvent.setup();
    renderViewer("deck.pptx");

    await screen.findByTestId("office-pdf-preview");

    await user.click(screen.getByRole("button", { name: "Save PDF" }));
    expect(projectOfficeSavePdfMock).toHaveBeenCalledWith("project-1", "deck.pptx", "render-1");
    expect(await screen.findByRole("status")).toHaveTextContent("Saved deck.pdf.");

    await user.click(screen.getByRole("button", { name: "Save a copy" }));
    expect(projectOfficeSaveOriginalMock).toHaveBeenCalledWith("project-1", "deck.pptx");
    await waitFor(() => expect(screen.queryByText(/Saved deck.pdf/)).not.toBeInTheDocument());
    expect(screen.queryByText("Could not save")).not.toBeInTheDocument();
  });

  it("reports a failed save", async () => {
    put("brief.docx", new Uint8Array([0, 1, 2]));
    projectOfficeSaveOriginalMock.mockRejectedValueOnce("That location cannot be written to.");
    const user = userEvent.setup();
    renderViewer("brief.docx");

    await user.click(await screen.findByRole("button", { name: "Save a copy" }));
    expect(await screen.findByText("Could not save")).toBeInTheDocument();
    expect(screen.getByText("The file could not be saved. That location cannot be written to.")).toBeInTheDocument();
  });

  it("can cancel a preview that is still being made, then ask for it again", async () => {
    put("slow.pptx", new Uint8Array([0, 1, 2]));
    const pending: { signal: AbortSignal | null } = { signal: null };
    projectOfficePreviewMock.mockImplementationOnce((_project: string, _path: string, signal: AbortSignal) => {
      pending.signal = signal;
      return new Promise<ReturnType<typeof PDF>>(() => {});
    });
    const user = userEvent.setup();
    renderViewer("slow.pptx");

    await showPreview(user);
    await user.click(await screen.findByRole("button", { name: "Cancel" }));
    expect(pending.signal?.aborted).toBe(true);
    await user.click(await screen.findByRole("button", { name: "Preview as PDF" }));
    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent("slow.pptx:37");
    expect(projectOfficePreviewMock).toHaveBeenCalledTimes(2);
  });

  it("offers to try again when the program is missing or the file cannot be opened", async () => {
    put("deck.pptx", new Uint8Array([0, 1, 2]));
    projectOfficePreviewMock.mockRejectedValueOnce(
      "This preview needs Microsoft Office, LibreOffice or WPS Office, and none of them was found.",
    );
    const user = userEvent.setup();
    renderViewer("deck.pptx");

    await showPreview(user);
    expect(await screen.findByText("Visual preview unavailable")).toBeInTheDocument();
    expect(
      screen.getByText(/This preview needs Microsoft Office, LibreOffice or WPS Office, and none of them was found/),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByTestId("office-pdf-preview")).toHaveTextContent("deck.pptx:37");
  });

  it("marks an LF edit dirty on its first character and clean again once it is undone", async () => {
    put("notes.txt", "first line\n");
    const user = userEvent.setup();
    renderViewer();
    await screen.findByText("first line");

    await user.click(screen.getByRole("button", { name: "Edit" }));
    const editor = screen.getByRole("textbox", { name: "File contents" });
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();

    await user.type(editor, "x");
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    await user.type(editor, "{Backspace}");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });
});
