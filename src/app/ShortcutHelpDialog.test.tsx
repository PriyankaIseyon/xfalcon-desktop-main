import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { resetPlatformForTests, setPlatform } from "../lib/platform";
import { ShortcutHelpDialog } from "./ShortcutHelpDialog";

afterEach(() => {
  cleanup();
  resetPlatformForTests();
});

describe("ShortcutHelpDialog", () => {
  it("stays out of the tree when closed", () => {
    const { container } = render(<ShortcutHelpDialog open={false} onClose={vi.fn()} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("opens naming itself, lists every shortcut with a description and platform-spelled keys", () => {
    setPlatform("windows");
    render(<ShortcutHelpDialog open onClose={vi.fn()} />);

    const dialog = screen.getByRole("dialog", { name: "Keyboard Shortcuts" });
    expect(within(dialog).getByText("Toggle Zen / Focus Mode")).toBeInTheDocument();
    expect(within(dialog).getByRole("img", { name: "Control plus Shift plus F" })).toHaveTextContent("Ctrl+Shift+F");
    expect(within(dialog).getByRole("img", { name: "/" })).toHaveTextContent("/");
  });

  it("spells the same shortcut in Apple's glyphs on macOS, not the other platform's word", () => {
    setPlatform("macos");
    render(<ShortcutHelpDialog open onClose={vi.fn()} />);

    expect(screen.getByRole("img", { name: "Command plus K" })).toHaveTextContent("⌘K");
  });

  it("closes on Escape and reports it through onClose, not by hiding itself", async () => {
    const onClose = vi.fn();
    const user = userEvent.setup();
    render(<ShortcutHelpDialog open onClose={onClose} />);

    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("gives the trailing keys a shrink floor of zero, so a long label truncates before the keys ever do", () => {
    render(<ShortcutHelpDialog open onClose={vi.fn()} />);

    const row = screen.getByText("Invoke slash commands (/clear, /model, /help)").closest("div")!;
    const shortcut = within(row).getByRole("img");
    expect(shortcut.className).toMatch(/\bshrink-0\b/);
    expect(row.querySelector(".truncate")).not.toBeNull();
  });
});
