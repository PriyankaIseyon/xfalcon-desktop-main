import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppearanceSettings } from "../../lib/generated/contracts";
import { installAppearance, resetAppearanceForTests } from "../../app/appearance";
import { Toaster } from "../../ui/Toaster";
import { resetToastsForTests } from "../../ui/toastStore";
import { AppearanceSection } from "./AppearanceSection";

const appearanceGetMock = vi.hoisted(() => vi.fn());
const appearanceSetMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  appearanceGet: appearanceGetMock,
  appearanceSet: appearanceSetMock,
}));

const stored: AppearanceSettings = {
  version: 1,
  theme: "dark",
  reading_size: "small",
  show_reasoning: true,
  show_follow_ups: true,
};

/**
 * Boots the app's appearance the way `main.tsx` does, then renders the section.
 *
 * The section reads the store rather than fetching for itself: it is no longer
 * the only reader — the transcript reads `show_reasoning` — and a second copy
 * loaded on mount is a second source of truth that can disagree with what the
 * rest of the app is rendering.
 */
async function boot() {
  await installAppearance();
  return render(<AppearanceSection />);
}

afterEach(() => {
  cleanup();
  resetToastsForTests();
  resetAppearanceForTests();
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.removeAttribute("data-font-size");
  appearanceGetMock.mockReset();
  appearanceSetMock.mockReset();
});

describe("AppearanceSection", () => {
  it("uses one grouped surface with two aligned preference rows", async () => {
    appearanceGetMock.mockResolvedValueOnce(stored);

    const { container } = await boot();

    expect(screen.getByRole("region", { name: "Appearance" })).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Appearance preferences" })).toBeInTheDocument();
    expect(container.querySelectorAll("[data-settings-row]")).toHaveLength(2);
    expect(screen.getByRole("radiogroup", { name: "Theme" })).toBeInTheDocument();
    expect(screen.getByRole("radiogroup", { name: "Reading size" })).toBeInTheDocument();
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
  });

  it("shows the stored choice as the selected pill", async () => {
    appearanceGetMock.mockResolvedValueOnce({
      version: 1,
      theme: "light",
      reading_size: "large",
      show_reasoning: true,
      show_follow_ups: true,
    });

    await boot();

    expect(screen.getByRole("radio", { name: "Light" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("radio", { name: "Large" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("radio", { name: "Dark" })).toHaveAttribute("aria-checked", "false");
  });

  it("applies a theme choice to the document before it is persisted", async () => {
    appearanceGetMock.mockResolvedValueOnce(stored);
    // Never resolves: the point is that the look changed anyway.
    appearanceSetMock.mockReturnValueOnce(new Promise(() => {}));
    const user = userEvent.setup();

    await boot();
    await user.click(screen.getByRole("radio", { name: "Light" }));

    expect(document.documentElement.dataset.theme).toBe("light");
    expect(appearanceSetMock).toHaveBeenCalledWith({
      version: 1,
      theme: "light",
      reading_size: "small",
      show_reasoning: true,
      show_follow_ups: true,
    });
  });

  it("a reading-size choice stamps the multiplier attribute and keeps the theme", async () => {
    appearanceGetMock.mockResolvedValueOnce(stored);
    appearanceSetMock.mockImplementation((next: AppearanceSettings) => Promise.resolve(next));
    const user = userEvent.setup();

    await boot();
    await user.click(screen.getByRole("radio", { name: "Medium" }));

    await waitFor(() => {
      expect(document.documentElement.dataset.fontSize).toBe("medium");
    });
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("reports an unsaved theme choice while leaving it applied", async () => {
    appearanceGetMock.mockResolvedValueOnce(stored);
    appearanceSetMock.mockRejectedValueOnce({ version: 1, code: "storage_unavailable" });
    const user = userEvent.setup();

    await installAppearance();
    render(
      <>
        <AppearanceSection />
        <Toaster />
      </>,
    );
    await user.click(screen.getByRole("radio", { name: "Light" }));

    expect(await screen.findByText("Display preferences applied but could not be saved.")).toBeInTheDocument();
    // The applied look stands: telling the user it was kept when it was not
    // is the one outcome worse than the failure.
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("persists rapid choices in the order they were applied", async () => {
    appearanceGetMock.mockResolvedValueOnce(stored);
    let finishFirst: (() => void) | undefined;
    appearanceSetMock
      .mockImplementationOnce(
        () =>
          new Promise<AppearanceSettings>((resolve) => {
            finishFirst = () => resolve({ ...stored, theme: "light" });
          }),
      )
      .mockImplementationOnce((next: AppearanceSettings) => Promise.resolve(next));
    const user = userEvent.setup();

    await boot();
    await user.click(screen.getByRole("radio", { name: "Light" }));
    await user.click(screen.getByRole("radio", { name: "Medium" }));

    expect(appearanceSetMock).toHaveBeenCalledTimes(1);
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(document.documentElement.dataset.fontSize).toBe("medium");

    finishFirst?.();
    await waitFor(() => expect(appearanceSetMock).toHaveBeenCalledTimes(2));
    expect(appearanceSetMock.mock.calls[1]?.[0]).toEqual({
      version: 1,
      theme: "light",
      reading_size: "medium",
      show_reasoning: true,
      show_follow_ups: true,
    });
  });

  it("falls back to the defaults when the stored preference cannot be read", async () => {
    appearanceGetMock.mockRejectedValueOnce(new Error("unreadable"));

    await boot();

    expect(screen.getByRole("radio", { name: "Light" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("radio", { name: "Small" })).toHaveAttribute("aria-checked", "true");
  });
});
