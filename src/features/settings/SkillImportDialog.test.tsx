import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SkillSourceGrantView, SkillView } from "../../lib/generated/contracts";
import type { SkillSourcesEvent } from "../../lib/native";
import { Toaster } from "../../ui/Toaster";
import { resetToastsForTests } from "../../ui/toastStore";
import { SkillImportDialog } from "./SkillImportDialog";

const skillImportMock = vi.hoisted(() => vi.fn());
const skillPickMock = vi.hoisted(() => vi.fn());
const onSkillSourcesMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  skillImport: skillImportMock,
  skillPick: skillPickMock,
  onSkillSources: onSkillSourcesMock,
}));

const dashboard: SkillView = {
  version: 1,
  name: "ida-dashboard",
  description: "Build self-contained interactive HTML dashboards.",
  declared_version: "0.0.0",
  enabled: true,
  content_hash: "a".repeat(64),
  bundled_files: 0,
};

/** What the backend grants for a picked or dropped selection: an id to import
 * with, how many sources it holds, and the names of the first few. */
function grant(grantId: string, count = 1, labels: string[] = ["ida-dashboard.md"]): SkillSourceGrantView {
  return { version: 1, grant_id: grantId, count, labels };
}

/** The last handler the dialog registered for window drops. */
let dropHandler: ((event: SkillSourcesEvent) => void) | null = null;
const unlisten = vi.hoisted(() => vi.fn());

afterEach(() => {
  cleanup();
  resetToastsForTests();
  skillImportMock.mockReset();
  skillPickMock.mockReset();
  onSkillSourcesMock.mockReset();
  unlisten.mockReset();
  dropHandler = null;
});

function armDrop() {
  onSkillSourcesMock.mockImplementation((handler: (event: SkillSourcesEvent) => void) => {
    dropHandler = handler;
    return Promise.resolve(unlisten);
  });
}

function renderDialog(overrides: Partial<React.ComponentProps<typeof SkillImportDialog>> = {}) {
  const onClose = vi.fn();
  const onImported = vi.fn();
  render(
    <>
      <SkillImportDialog open onClose={onClose} onImported={onImported} {...overrides} />
      <Toaster />
    </>,
  );
  return { onClose, onImported };
}

describe("SkillImportDialog", () => {
  it("states what it accepts, so the reader is not guessing from a button label", async () => {
    armDrop();
    renderDialog();

    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    expect(screen.getByText(/\.skill packages and skill folders, with every bundled file/)).toBeInTheDocument();
    expect(screen.getByText(/Single \.md skill files/)).toBeInTheDocument();
    expect(screen.getByText(/ZIP archives, including nested skill folders and \.skill packages/i)).toBeInTheDocument();
    expect(screen.getByText(/Folders, including subfolders/i)).toBeInTheDocument();
    // Name is identity, and that has consequences the user should know before
    // importing over something.
    expect(screen.getByText(/Importing an existing skill name replaces its files/i)).toBeInTheDocument();
  });

  it("a drop imports the grant it arrives with — files, archives and folders together", async () => {
    armDrop();
    skillImportMock.mockResolvedValueOnce({ version: 1, imported: [dashboard], failed: [] });
    const { onImported } = renderDialog();
    await screen.findByRole("dialog");
    await waitFor(() => expect(dropHandler).not.toBeNull());

    // One grant over the whole mix: no picker can do this, which is why drop is
    // the primary path. The renderer never sees the paths behind it.
    await act(async () => {
      dropHandler?.({
        kind: "dropped",
        grant: grant("g-drop", 3, ["one.md", "library.zip", "skills"]),
      });
    });

    await waitFor(() => expect(skillImportMock).toHaveBeenCalledWith("g-drop"));
    expect(onImported).toHaveBeenCalled();
  });

  it("does not start a second import while one is running, even for another drop", async () => {
    armDrop();
    let finish!: (value: { version: 1; imported: SkillView[]; failed: [] }) => void;
    skillImportMock.mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    renderDialog();
    await screen.findByRole("dialog");
    await waitFor(() => expect(dropHandler).not.toBeNull());

    await act(async () => {
      dropHandler?.({ kind: "dropped", grant: grant("g-first") });
    });
    await act(async () => {
      dropHandler?.({ kind: "dropped", grant: grant("g-second") });
    });

    expect(skillImportMock).toHaveBeenCalledTimes(1);
    expect(skillImportMock).toHaveBeenCalledWith("g-first");
    await act(async () => finish({ version: 1, imported: [], failed: [] }));
  });

  it("says a drop that is too large to grant was not imported, instead of doing nothing", async () => {
    armDrop();
    renderDialog();
    await screen.findByRole("dialog");
    await waitFor(() => expect(dropHandler).not.toBeNull());

    await act(async () => {
      dropHandler?.({ kind: "rejected", code: "selection_too_large" });
    });

    expect(
      await screen.findByText("That selection has too many files. Choose fewer, or a folder."),
    ).toBeInTheDocument();
    expect(skillImportMock).not.toHaveBeenCalled();
  });

  it("says a picked selection was not imported when it is too large to grant", async () => {
    armDrop();
    skillPickMock.mockRejectedValueOnce({ version: 1, code: "selection_too_large" });
    const user = userEvent.setup();
    renderDialog();

    await user.click(await screen.findByRole("button", { name: /files or archive/i }));

    expect(
      await screen.findByText("That selection has too many files. Choose fewer, or a folder."),
    ).toBeInTheDocument();
    expect(skillImportMock).not.toHaveBeenCalled();
  });

  it("tells the person when a picked selection arrives during a drop import, rather than dropping it", async () => {
    armDrop();
    let choosePicked!: (value: SkillSourceGrantView) => void;
    skillPickMock.mockReturnValueOnce(
      new Promise((resolve) => {
        choosePicked = resolve;
      }),
    );
    let finishDrop!: (value: { version: 1; imported: SkillView[]; failed: [] }) => void;
    skillImportMock.mockReturnValueOnce(
      new Promise((resolve) => {
        finishDrop = resolve;
      }),
    );
    const user = userEvent.setup();
    renderDialog();
    await screen.findByRole("dialog");
    await waitFor(() => expect(dropHandler).not.toBeNull());

    // The picker is open while a drop import starts; the picked grant arrives after.
    await user.click(screen.getByRole("button", { name: /files or archive/i }));
    await act(async () => {
      dropHandler?.({ kind: "dropped", grant: grant("g-drop") });
    });
    await act(async () => choosePicked(grant("g-picked")));

    expect(
      await screen.findByText("Wait for the current import to finish, then choose the files again."),
    ).toBeInTheDocument();
    expect(skillImportMock).toHaveBeenCalledTimes(1);
    expect(skillImportMock).toHaveBeenCalledWith("g-drop");
    await act(async () => finishDrop({ version: 1, imported: [], failed: [] }));
  });

  it("reflects a drag over the window and stops when it leaves", async () => {
    armDrop();
    renderDialog();
    await screen.findByRole("dialog");
    await waitFor(() => expect(dropHandler).not.toBeNull());

    await act(async () => {
      dropHandler?.({ kind: "over" });
    });
    expect(await screen.findByText("Release to import")).toBeInTheDocument();

    await act(async () => {
      dropHandler?.({ kind: "leave" });
    });
    expect(await screen.findByText("Drop skills here")).toBeInTheDocument();
  });

  it("stops listening for drops once closed, so a later drop imports nothing", async () => {
    armDrop();
    const { rerender } = render(<SkillImportDialog open onClose={vi.fn()} onImported={vi.fn()} />);
    await waitFor(() => expect(dropHandler).not.toBeNull());

    rerender(<SkillImportDialog open={false} onClose={vi.fn()} onImported={vi.fn()} />);

    await waitFor(() => expect(unlisten).toHaveBeenCalled());
  });

  it("the file chooser takes several files and an archive among them, as one grant", async () => {
    armDrop();
    skillPickMock.mockResolvedValueOnce(grant("g-files", 3, ["a.md", "b.skill", "c.zip"]));
    skillImportMock.mockResolvedValueOnce({ version: 1, imported: [dashboard], failed: [] });
    const user = userEvent.setup();
    renderDialog();

    await user.click(await screen.findByRole("button", { name: /files or archive/i }));

    await waitFor(() => {
      expect(skillPickMock).toHaveBeenCalledWith("files");
      expect(skillImportMock).toHaveBeenCalledWith("g-files");
    });
  });

  it("the folder chooser hands over the folder itself and lets the backend walk it", async () => {
    armDrop();
    skillPickMock.mockResolvedValueOnce(grant("g-folder", 1, ["skills"]));
    skillImportMock.mockResolvedValueOnce({ version: 1, imported: [dashboard], failed: [] });
    const user = userEvent.setup();
    renderDialog();

    await user.click(await screen.findByRole("button", { name: /^folder$/i }));

    await waitFor(() => {
      expect(skillPickMock).toHaveBeenCalledWith("folder");
      expect(skillImportMock).toHaveBeenCalledWith("g-folder");
    });
  });

  it("cancelling a picker imports nothing", async () => {
    armDrop();
    skillPickMock.mockResolvedValueOnce(null);
    const user = userEvent.setup();
    renderDialog();

    await user.click(await screen.findByRole("button", { name: /files or archive/i }));

    await waitFor(() => expect(skillPickMock).toHaveBeenCalledWith("files"));
    expect(skillImportMock).not.toHaveBeenCalled();
  });

  it("reports a picker failure instead of treating it as a cancellation", async () => {
    armDrop();
    skillPickMock.mockRejectedValueOnce(new Error("picker unavailable"));
    const user = userEvent.setup();
    renderDialog();

    await user.click(await screen.findByRole("button", { name: /files or archive/i }));

    expect(await screen.findByText("The system picker could not be opened.")).toBeInTheDocument();
    expect(skillImportMock).not.toHaveBeenCalled();
  });

  it("closes itself when everything imported, so success needs no dismissing", async () => {
    armDrop();
    skillPickMock.mockResolvedValueOnce(grant("g-good"));
    skillImportMock.mockResolvedValueOnce({ version: 1, imported: [dashboard], failed: [] });
    const user = userEvent.setup();
    const { onClose } = renderDialog();

    await user.click(await screen.findByRole("button", { name: /files or archive/i }));

    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("stays open holding each reason when a named file would not import", async () => {
    armDrop();
    skillPickMock.mockResolvedValueOnce(grant("g-mixed", 2, ["good.md", "bad.md"]));
    skillImportMock.mockResolvedValueOnce({
      version: 1,
      imported: [dashboard],
      failed: [{ path: "bad.md", reason: "frontmatter delimiters not found" }],
    });
    const user = userEvent.setup();
    const { onClose, onImported } = renderDialog();

    await user.click(await screen.findByRole("button", { name: /files or archive/i }));

    // The failure names the file only; the backend never sends its folder.
    expect(await screen.findByText("bad.md")).toBeInTheDocument();
    expect(screen.queryByText(/\/tmp/)).not.toBeInTheDocument();
    expect(screen.getByText(/frontmatter delimiters not found/)).toBeInTheDocument();
    // The valid sibling still landed, and the reasons are not thrown away by
    // closing over them.
    expect(onImported).toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("says a choice that expired is not usable any more, instead of importing it", async () => {
    armDrop();
    skillPickMock.mockResolvedValueOnce(grant("g-stale"));
    skillImportMock.mockRejectedValueOnce({ version: 1, code: "invalid_choice" });
    const user = userEvent.setup();
    renderDialog();

    await user.click(await screen.findByRole("button", { name: /files or archive/i }));

    expect(await screen.findByText("That choice expired. Choose it again.")).toBeInTheDocument();
  });

  it("turns a typed failure into a sentence rather than showing a code", async () => {
    armDrop();
    skillPickMock.mockResolvedValueOnce(grant("g-typed"));
    skillImportMock.mockRejectedValueOnce({ version: 1, code: "storage_unavailable" });
    const user = userEvent.setup();
    renderDialog();

    await user.click(await screen.findByRole("button", { name: /files or archive/i }));

    expect(
      await screen.findByText("The skills folder on this device could not be read or written."),
    ).toBeInTheDocument();
  });

  it("a window that cannot report drops still offers both pickers", async () => {
    // Losing the drop listener must not take the dialog down with it.
    onSkillSourcesMock.mockRejectedValueOnce(new Error("no drag-drop here"));
    renderDialog();

    expect(await screen.findByRole("button", { name: /files or archive/i })).toBeEnabled();
    expect(screen.getByRole("button", { name: /^folder$/i })).toBeEnabled();
  });
});
