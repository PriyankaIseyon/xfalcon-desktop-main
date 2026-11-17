import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DestinationNav, SidebarRail } from "./WorkspaceNav";

afterEach(cleanup);

describe("DestinationNav", () => {
  it("moves one persistent highlight between primary destinations", () => {
    const onNavigate = vi.fn();
    const { container, rerender } = render(<DestinationNav chatReady view="chat" onNavigate={onNavigate} />);
    const group = container.querySelector(".app-sidebar__destinations");
    const highlight = container.querySelector(".nav-destination__highlight");

    expect(group).toHaveAttribute("data-active-index", "0");
    expect(highlight).not.toBeNull();

    rerender(<DestinationNav chatReady view="projects" onNavigate={onNavigate} />);

    expect(group).toHaveAttribute("data-active-index", "1");
    expect(container.querySelector(".nav-destination__highlight")).toBe(highlight);
  });

  it("disables the Chats '+' while a chat cannot start, and enables it once one can", () => {
    const onCreate = vi.fn();
    const { rerender } = render(
      <DestinationNav chatReady={false} view="projects" onCreate={onCreate} onNavigate={vi.fn()} />,
    );
    const newChat = screen.getByRole("button", { name: "Start a new chat" });

    expect(newChat).toBeDisabled();
    expect(screen.getByRole("button", { name: "Create a project" })).toBeEnabled();
    fireEvent.click(newChat);
    expect(onCreate).not.toHaveBeenCalled();

    rerender(<DestinationNav chatReady view="projects" onCreate={onCreate} onNavigate={vi.fn()} />);
    expect(newChat).toBeEnabled();
  });
});

describe("SidebarRail", () => {
  it("is a window drag region only when the shell says the window drags", () => {
    const railProps = { account: null, view: "chat" as const, onNavigate: vi.fn(), onNewChat: vi.fn() };
    const { rerender } = render(<SidebarRail {...railProps} windowDrag />);
    expect(screen.getByRole("navigation", { name: "Workspace" })).toHaveAttribute("data-tauri-drag-region");

    rerender(<SidebarRail {...railProps} />);
    expect(screen.getByRole("navigation", { name: "Workspace" })).not.toHaveAttribute("data-tauri-drag-region");
  });
});
