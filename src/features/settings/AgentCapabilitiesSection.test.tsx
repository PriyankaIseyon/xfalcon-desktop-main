import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppearanceSettings } from "../../lib/generated/contracts";
import { installAppearance, resetAppearanceForTests } from "../../app/appearance";
import { Toaster } from "../../ui/Toaster";
import { resetToastsForTests } from "../../ui/toastStore";
import { AgentCapabilitiesSection } from "./AgentCapabilitiesSection";

const appearanceGetMock = vi.hoisted(() => vi.fn());
const appearanceSetMock = vi.hoisted(() => vi.fn());
const skillListMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  appearanceGet: appearanceGetMock,
  appearanceSet: appearanceSetMock,
  skillList: skillListMock,
  skillDelete: vi.fn(),
  skillSetEnabled: vi.fn(),
}));

const stored: AppearanceSettings = {
  version: 1,
  theme: "dark",
  reading_size: "small",
  show_reasoning: true,
  show_follow_ups: true,
};

async function boot() {
  await installAppearance();
  return render(<AgentCapabilitiesSection />);
}

afterEach(() => {
  cleanup();
  resetToastsForTests();
  resetAppearanceForTests();
  appearanceGetMock.mockReset();
  appearanceSetMock.mockReset();
  skillListMock.mockReset();
});

describe("AgentCapabilitiesSection", () => {
  it("names the section and puts the two switches above the skills block in one surface", async () => {
    appearanceGetMock.mockResolvedValueOnce(stored);
    skillListMock.mockResolvedValueOnce({ skills: [] });

    await boot();

    const region = screen.getByRole("region", { name: "Agent capabilities" });
    const surface = within(region).getByRole("group", { name: "Agent capability settings" });
    const skills = within(surface).getByRole("group", { name: "Skills" });
    expect(skills).toHaveAttribute("data-settings-group-nested");
    expect(surface).not.toHaveAttribute("data-settings-group-nested");

    const reasoning = screen.getByRole("switch", { name: "Model reasoning" });
    const followUps = screen.getByRole("switch", { name: "Try next suggestions" });
    expect(reasoning.closest("[data-settings-row]")).toHaveAttribute("id", "settings-show-reasoning-row");
    expect(followUps.closest("[data-settings-row]")).toHaveAttribute("id", "settings-show-follow-ups-row");
    expect(reasoning.compareDocumentPosition(skills) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(followUps.compareDocumentPosition(skills) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("hiding reasoning persists the whole preference set, not just that field", async () => {
    appearanceGetMock.mockResolvedValueOnce(stored);
    appearanceSetMock.mockImplementation((next: AppearanceSettings) => Promise.resolve(next));
    skillListMock.mockResolvedValueOnce({ skills: [] });
    const user = userEvent.setup();

    await boot();
    const toggle = screen.getByRole("switch", { name: /Model reasoning/ });
    expect(toggle).toHaveAttribute("aria-checked", "true");

    await user.click(toggle);

    expect(appearanceSetMock).toHaveBeenCalledWith({
      version: 1,
      theme: "dark",
      reading_size: "small",
      show_reasoning: false,
      show_follow_ups: true,
    });
    await waitFor(() => {
      expect(screen.getByRole("switch", { name: /Model reasoning/ })).toHaveAttribute("aria-checked", "false");
    });
  });

  it("shows a stored hidden preference as off", async () => {
    appearanceGetMock.mockResolvedValueOnce({ ...stored, show_reasoning: false });
    skillListMock.mockResolvedValueOnce({ skills: [] });

    await boot();

    expect(screen.getByRole("switch", { name: /Model reasoning/ })).toHaveAttribute("aria-checked", "false");
  });

  it("reports an unsaved follow-ups choice while leaving it applied", async () => {
    appearanceGetMock.mockResolvedValueOnce(stored);
    appearanceSetMock.mockRejectedValueOnce({ version: 1, code: "storage_unavailable" });
    skillListMock.mockResolvedValueOnce({ skills: [] });
    const user = userEvent.setup();

    await installAppearance();
    render(
      <>
        <AgentCapabilitiesSection />
        <Toaster />
      </>,
    );
    await user.click(screen.getByRole("switch", { name: "Try next suggestions" }));

    expect(await screen.findByText("Display preferences applied but could not be saved.")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Try next suggestions" })).toHaveAttribute("aria-checked", "false");
  });

  it("falls back to the defaults when the stored preference cannot be read", async () => {
    appearanceGetMock.mockRejectedValueOnce(new Error("unreadable"));
    skillListMock.mockResolvedValueOnce({ skills: [] });

    await boot();

    expect(screen.getByRole("switch", { name: /Model reasoning/ })).toHaveAttribute("aria-checked", "false");
  });
});
