import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SkillSourceGrantView, SkillView } from "../../lib/generated/contracts";
import { Toaster } from "../../ui/Toaster";
import { resetToastsForTests } from "../../ui/toastStore";
import { SkillsSection } from "./SkillsSection";

const skillListMock = vi.hoisted(() => vi.fn());
const skillImportMock = vi.hoisted(() => vi.fn());
const skillSetEnabledMock = vi.hoisted(() => vi.fn());
const skillDeleteMock = vi.hoisted(() => vi.fn());
const skillPickMock = vi.hoisted(() => vi.fn());
const onSkillSourcesMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  skillList: skillListMock,
  skillImport: skillImportMock,
  skillSetEnabled: skillSetEnabledMock,
  skillDelete: skillDeleteMock,
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

afterEach(() => {
  cleanup();
  resetToastsForTests();
  skillListMock.mockReset();
  skillImportMock.mockReset();
  skillSetEnabledMock.mockReset();
  skillDeleteMock.mockReset();
  skillPickMock.mockReset();
  onSkillSourcesMock.mockReset();
  onSkillSourcesMock.mockResolvedValue(() => {});
});

describe("SkillsSection", () => {
  it("offers exactly one way in, so the same intent is not spelled four ways", async () => {
    // The regression this pins: an import button in the header AND a pair in
    // the empty state put four controls on screen for two actions.
    skillListMock.mockResolvedValueOnce({ version: 1, skills: [] });

    render(<SkillsSection />);

    expect(await screen.findByText("No skills imported")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /import skills/i })).toHaveLength(1);
    expect(screen.queryByRole("button", { name: /import folder/i })).not.toBeInTheDocument();
  });

  it("the one button opens the dialog that explains the rules before offering the inputs", async () => {
    skillListMock.mockResolvedValueOnce({ version: 1, skills: [] });
    const user = userEvent.setup();

    render(<SkillsSection />);
    await screen.findByText("No skills imported");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /import skills/i }));

    const dialog = await screen.findByRole("dialog");
    expect(dialog).toBeInTheDocument();
    // The rules are stated, not left to a button label.
    expect(screen.getByText(/Folders, including subfolders/i)).toBeInTheDocument();
  });

  it("re-reads the library after the dialog imports something", async () => {
    skillListMock
      .mockResolvedValueOnce({ version: 1, skills: [] })
      .mockResolvedValueOnce({ version: 1, skills: [dashboard] });
    const picked: SkillSourceGrantView = {
      version: 1,
      grant_id: "g-dashboard",
      count: 1,
      labels: ["ida-dashboard.md"],
    };
    skillPickMock.mockResolvedValueOnce(picked);
    skillImportMock.mockResolvedValueOnce({ version: 1, imported: [dashboard], failed: [] });
    const user = userEvent.setup();

    render(<SkillsSection />);
    await screen.findByText("No skills imported");
    await user.click(screen.getByRole("button", { name: /import skills/i }));
    await user.click(await screen.findByRole("button", { name: /files or archive/i }));

    // A grant crosses the boundary, not a path or bytes.
    await waitFor(() => {
      expect(skillImportMock).toHaveBeenCalledWith("g-dashboard");
    });
    expect(await screen.findByText("ida-dashboard")).toBeInTheDocument();
  });

  it("says how many files a skill bundles, and says nothing for a single-file skill", async () => {
    skillListMock.mockResolvedValueOnce({
      version: 1,
      skills: [{ ...dashboard, name: "ida-viz", bundled_files: 7 }, dashboard],
    });

    render(<SkillsSection />);
    await screen.findByText("ida-viz");

    expect(screen.getByText(/Available for agent tasks\. · 7 bundled files/)).toBeInTheDocument();
    expect(screen.getAllByText(/bundled file/)).toHaveLength(1);
  });

  it("turning a skill off keeps it listed", async () => {
    skillListMock.mockResolvedValueOnce({ version: 1, skills: [dashboard] });
    skillSetEnabledMock.mockResolvedValueOnce({ ...dashboard, enabled: false });
    const user = userEvent.setup();

    render(<SkillsSection />);
    await screen.findByText("ida-dashboard");
    await user.click(screen.getByRole("switch"));

    await waitFor(() => {
      expect(skillSetEnabledMock).toHaveBeenCalledWith("ida-dashboard", false);
    });
    expect(screen.getByText("ida-dashboard")).toBeInTheDocument();
    expect(await screen.findByText("Stored but disabled.")).toBeInTheDocument();
  });

  it("confirms removal, then re-reads the library rather than trusting its own state", async () => {
    skillListMock
      .mockResolvedValueOnce({ version: 1, skills: [dashboard] })
      .mockResolvedValueOnce({ version: 1, skills: [] });
    skillDeleteMock.mockResolvedValueOnce(undefined);
    const user = userEvent.setup();

    render(<SkillsSection />);
    await screen.findByText("ida-dashboard");
    await user.click(screen.getByRole("button", { name: /remove ida-dashboard/i }));

    expect(skillDeleteMock).not.toHaveBeenCalled();
    const dialog = screen.getByRole("alertdialog", { name: "Remove “ida-dashboard”?" });
    expect(dialog).toHaveTextContent("its imported files will be removed from this device");
    await user.click(screen.getByRole("button", { name: "Remove skill" }));

    await waitFor(() => {
      expect(skillDeleteMock).toHaveBeenCalledWith("ida-dashboard");
    });
    expect(await screen.findByText("No skills imported")).toBeInTheDocument();
  });

  it("retains a failed removal for retry with the skill still present", async () => {
    skillListMock.mockResolvedValueOnce({ version: 1, skills: [dashboard] });
    skillDeleteMock.mockRejectedValueOnce({ version: 1, code: "storage_unavailable" });
    const user = userEvent.setup();

    render(<SkillsSection />);
    await screen.findByText("ida-dashboard");
    await user.click(screen.getByRole("button", { name: /remove ida-dashboard/i }));
    await user.click(screen.getByRole("button", { name: "Remove skill" }));

    const dialog = await screen.findByRole("alertdialog", { name: "Remove “ida-dashboard”?" });
    expect(dialog).toHaveTextContent("The skills folder on this device could not be read or written.");
    expect(screen.getByText("ida-dashboard")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove skill" })).toBeEnabled();
  });

  it("turns a typed failure into a sentence rather than showing a code", async () => {
    skillListMock.mockRejectedValueOnce({ version: 1, code: "storage_unavailable" });

    render(
      <>
        <SkillsSection />
        <Toaster />
      </>,
    );

    expect(
      await screen.findByText("The skills folder on this device could not be read or written."),
    ).toBeInTheDocument();
  });
});
