import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, SidebarResizer } from "./SidebarResizer";

/** The shell as AppShell draws it: the title bar's toggle beside the sidebar. */
function Shell({ sidebarOpen = true }: { sidebarOpen?: boolean }) {
  return (
    <div className="app-shell" data-testid="shell">
      <header className="app-titlebar">
        <button aria-controls="app-sidebar" type="button">
          Hide sidebar
        </button>
      </header>
      <aside id="app-sidebar">{sidebarOpen ? <SidebarResizer /> : null}</aside>
    </div>
  );
}

function renderInShell() {
  const { rerender } = render(<Shell />);
  return {
    handle: screen.getByRole("separator", { name: "Resize sidebar" }),
    shellWidth: () => screen.getByTestId("shell").style.getPropertyValue("--xf-sidebar-width"),
    collapse: () => rerender(<Shell sidebarOpen={false} />),
  };
}

describe("SidebarResizer", () => {
  afterEach(() => {
    // Back to the default so each test starts from it: the width is kept for the
    // session at module scope. This suite registers no automatic cleanup.
    cleanup();
    const { handle } = renderInShell();
    fireEvent.keyDown(handle, { key: "Enter" });
    cleanup();
  });

  it("is a focusable vertical separator that writes the width onto the shell", () => {
    const { handle, shellWidth } = renderInShell();
    expect(handle).toHaveAttribute("aria-orientation", "vertical");
    expect(handle).toHaveAttribute("aria-controls", "app-sidebar");
    expect(handle).toHaveAttribute("tabindex", "0");
    expect(handle).toHaveAttribute("aria-valuenow", String(SIDEBAR_DEFAULT_WIDTH));
    expect(shellWidth()).toBe(`${SIDEBAR_DEFAULT_WIDTH}px`);
  });

  it("steps with the arrow keys, jumps with Home and End, and resets with Enter", () => {
    const { handle, shellWidth } = renderInShell();
    fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(handle).toHaveAttribute("aria-valuenow", String(SIDEBAR_DEFAULT_WIDTH + 8));
    fireEvent.keyDown(handle, { key: "ArrowLeft", shiftKey: true });
    expect(handle).toHaveAttribute("aria-valuenow", String(SIDEBAR_DEFAULT_WIDTH + 8 - 32));
    fireEvent.keyDown(handle, { key: "End" });
    expect(handle).toHaveAttribute("aria-valuenow", String(SIDEBAR_MAX_WIDTH));
    fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(handle).toHaveAttribute("aria-valuenow", String(SIDEBAR_MAX_WIDTH));
    fireEvent.keyDown(handle, { key: "Home" });
    expect(handle).toHaveAttribute("aria-valuenow", String(SIDEBAR_MIN_WIDTH));
    expect(shellWidth()).toBe(`${SIDEBAR_MIN_WIDTH}px`);
    fireEvent.keyDown(handle, { key: "Enter" });
    expect(handle).toHaveAttribute("aria-valuenow", String(SIDEBAR_DEFAULT_WIDTH));
  });

  it("resets on double-click", () => {
    const { handle } = renderInShell();
    fireEvent.keyDown(handle, { key: "End" });
    fireEvent.doubleClick(handle);
    expect(handle).toHaveAttribute("aria-valuenow", String(SIDEBAR_DEFAULT_WIDTH));
  });

  it("keeps the chosen width when the sidebar is collapsed and opened again", () => {
    const first = renderInShell();
    fireEvent.keyDown(first.handle, { key: "End" });
    cleanup();
    const second = renderInShell();
    expect(second.handle).toHaveAttribute("aria-valuenow", String(SIDEBAR_MAX_WIDTH));
  });

  it("moves focus to the title bar toggle when collapsing removes the focused handle", async () => {
    const { handle, collapse } = renderInShell();
    const toggle = screen.getByRole("button", { name: "Hide sidebar" });
    handle.focus();
    collapse();
    expect(handle).not.toBeInTheDocument();
    await waitFor(() => expect(toggle).toHaveFocus());
  });
});
