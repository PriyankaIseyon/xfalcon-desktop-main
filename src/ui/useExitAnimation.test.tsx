import React from "react";
import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { Menu } from "./Menu";
import { Modal } from "./Modal";
import { Tooltip } from "./Tooltip";

// jsdom has no Web Animations, and without them `useExitAnimation` leaves
// nothing behind -- the right answer there, and the reason every other suite
// sees a closed surface gone at once. These tests give the platform what a
// WebView has: `getAnimations()` on every element, reporting the exit the
// stylesheet would start on a leaving copy.
type ExitMode = "finite" | "infinite" | "none";

let mode: ExitMode = "finite";
let finishExits: () => void = () => {};

function installAnimations() {
  let release: () => void = () => {};
  const finished = new Promise<void>((resolve) => {
    release = resolve;
  });
  finishExits = release;
  Object.defineProperty(Element.prototype, "getAnimations", {
    configurable: true,
    value(this: Element) {
      if (mode === "none" || !this.hasAttribute("data-xf-leaving")) return [];
      const endTime = mode === "infinite" ? Infinity : 500;
      return [{ effect: { target: this, getComputedTiming: () => ({ endTime }) }, finished }];
    },
  });
}

/** The commit has landed and the copy has been put back. */
async function afterCommit() {
  await act(async () => {
    await Promise.resolve();
  });
}

function leaving(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>("[data-xf-leaving]"));
}

beforeEach(() => {
  mode = "finite";
  installAnimations();
});

afterEach(async () => {
  // Unmounting whatever a test left open leaves its own copy one microtask on.
  cleanup();
  await Promise.resolve();
  for (const node of leaving()) node.remove();
  document.getElementById("root")?.remove();
  delete (Element.prototype as { getAnimations?: unknown }).getAnimations;
});

function RowMenu() {
  return (
    <Menu trigger={<button type="button">Row actions</button>}>
      <Menu.Item onSelect={() => {}}>Rename</Menu.Item>
    </Menu>
  );
}

describe("useExitAnimation", () => {
  it("leaves an inert, unnamed copy of a closed menu until its exit finishes", async () => {
    const user = userEvent.setup();
    render(<RowMenu />);

    await user.click(screen.getByRole("button", { name: "Row actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Rename" }));
    await afterCommit();

    // The live menu is gone at once: nothing can reach or announce the copy.
    expect(screen.queryByRole("menu")).toBeNull();
    const [copy, ...others] = leaving();
    expect(others).toHaveLength(0);
    expect(copy).toHaveAttribute("role", "menu");
    expect(copy).toHaveTextContent("Rename");
    expect(copy.inert).toBe(true);
    expect(copy).toHaveAttribute("aria-hidden", "true");
    expect(copy.querySelector("[id]")).toBeNull();
    expect(copy.id).toBe("");
    expect(screen.getByRole("button", { name: "Row actions" })).toHaveAttribute("aria-expanded", "false");

    await act(async () => {
      finishExits();
      await Promise.resolve();
    });
    expect(leaving()).toHaveLength(0);
  });

  it("plays the exit of a dialog its caller drops, with the text it last showed", async () => {
    const user = userEvent.setup();
    function Caller() {
      const [target, setTarget] = React.useState<string | null>("Quarterly model");
      return target == null ? null : (
        <Modal open title={`Delete “${target}”?`} onClose={() => setTarget(null)}>
          <input aria-label="Reason" name="reason" defaultValue="" />
        </Modal>
      );
    }
    // The app root the focus trap marks inert, as `main.tsx` mounts into.
    const appRoot = document.body.appendChild(document.createElement("div"));
    appRoot.id = "root";
    render(<Caller />, { container: appRoot });
    expect(appRoot).toHaveAttribute("inert");

    await user.type(screen.getByRole("textbox", { name: "Reason" }), "stale");
    await user.click(screen.getByRole("button", { name: "Close" }));
    await afterCommit();

    expect(screen.queryByRole("dialog")).toBeNull();
    const [copy] = leaving();
    expect(copy).toHaveClass("dialog-shell");
    expect(copy).toHaveTextContent("Delete “Quarterly model”?");
    // What the reader typed, not the markup's empty default, and unnamed so it
    // joins no live form or radio group.
    const field = copy.querySelector("input");
    expect(field?.value).toBe("stale");
    expect(field).not.toHaveAttribute("name");
    // The app behind is live while the copy fades.
    expect(appRoot).not.toHaveAttribute("inert");
    expect(copy).not.toHaveAttribute("aria-modal");
  });

  it("leaves nothing when no exit runs, as under reduced motion", async () => {
    mode = "none";
    const user = userEvent.setup();
    render(<RowMenu />);

    await user.click(screen.getByRole("button", { name: "Row actions" }));
    await user.keyboard("{Escape}");
    await afterCommit();

    expect(screen.queryByRole("menu")).toBeNull();
    expect(leaving()).toHaveLength(0);
  });

  it("does not wait on an animation that never ends", async () => {
    mode = "infinite";
    const user = userEvent.setup();
    render(<RowMenu />);

    await user.click(screen.getByRole("button", { name: "Row actions" }));
    await user.keyboard("{Escape}");
    await afterCommit();

    expect(leaving()).toHaveLength(0);
  });

  it("leaves nothing behind for a StrictMode re-attach of a surface that stays open", async () => {
    const user = userEvent.setup();
    render(
      <React.StrictMode>
        <RowMenu />
      </React.StrictMode>,
    );

    await user.click(screen.getByRole("button", { name: "Row actions" }));
    await afterCommit();

    expect(screen.getByRole("menu")).toBeInTheDocument();
    expect(leaving()).toHaveLength(0);
  });

  it("fades a tooltip out from the side it opened on", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip label="Copy message" side="bottom">
        <button type="button">Copy</button>
      </Tooltip>,
    );

    await user.tab();
    expect(screen.getByRole("tooltip")).toHaveAttribute("data-side", "bottom");
    await user.tab();
    await afterCommit();

    expect(screen.queryByRole("tooltip")).toBeNull();
    const [copy] = leaving();
    expect(copy).toHaveClass("xf-overlay-fade");
    expect(copy).toHaveAttribute("data-side", "bottom");
  });
});
