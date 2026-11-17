import React from "react";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setPlatform, resetPlatformForTests } from "../lib/platform";
import { Avatar } from "./Avatar";
import { Badge } from "./Badge";
import { Chip } from "./Chip";
import { Divider } from "./Divider";
import { Field, Input } from "./Field";
import { Eyebrow, SectionTitle } from "./Heading";
import { Modal } from "./Modal";
import { Kbd, Shortcut } from "./Kbd";
import { List, ListItem } from "./List";
import { NavBar } from "./NavBar";
import { Pagination } from "./Pagination";
import { Sidebar } from "./Sidebar";
import { XFalconMark } from "./XFalconMark";

afterEach(() => {
  cleanup();
  resetPlatformForTests();
});

// ---------------------------------------------------------------------------
// P31-P42: structure and display.
//
// These primitives carry no state of their own, so the failure they invite is
// quieter than a broken control: a name that is never spoken, a label that
// names a key half the readers do not have, a row that looks like a control
// and is not. This file pins what each of them says about itself.
// ---------------------------------------------------------------------------

describe("Shortcut (P41)", () => {
  it("prints the platform's own modifier rather than naming both", () => {
    // The cheat sheet used to print `⌘/Ctrl` to everyone, which names a key
    // half its readers do not have; the sidebar said `Ctrl K` and the rail
    // said `Ctrl+K` for the very same shortcut.
    setPlatform("windows");
    const { rerender } = render(<Shortcut keys={["Mod", "Shift", "F"]} />);
    let shortcut = screen.getByRole("img");
    expect(shortcut).toHaveTextContent("Ctrl+Shift+F");
    expect(shortcut.textContent).not.toContain("⌘");

    setPlatform("macos");
    rerender(<Shortcut keys={["Mod", "Shift", "F"]} />);
    shortcut = screen.getByRole("img");
    expect(shortcut).toHaveTextContent("⌘⇧F");
    // Apple runs the glyphs together; a plus between them is Windows spelling.
    expect(shortcut.textContent).not.toContain("+");
    expect(shortcut.textContent).not.toContain("Ctrl");
  });

  it("speaks words where it prints glyphs", () => {
    // `⌘` is "place of interest sign" to a screen reader, and a run of
    // separate caps is a string of disconnected letters. One name, in words.
    setPlatform("macos");
    render(<Shortcut keys={["Mod", "K"]} />);
    expect(screen.getByRole("img", { name: "Command plus K" })).toBeInTheDocument();
    for (const cap of document.querySelectorAll("kbd")) {
      expect(cap).toHaveAttribute("aria-hidden", "true");
    }
  });

  it("names the non-Apple modifier Control, on Windows and before bootstrap answers", () => {
    setPlatform("windows");
    const { rerender } = render(<Shortcut keys={["Mod", "K"]} />);
    expect(screen.getByRole("img", { name: "Control plus K" })).toBeInTheDocument();

    // `unknown` is what every consumer renders for the moment before bootstrap
    // resolves, and words are legible on any platform if the guess is wrong.
    resetPlatformForTests();
    rerender(<Shortcut keys={["Mod", "K"]} />);
    expect(screen.getByRole("img", { name: "Control plus K" })).toHaveTextContent("Ctrl+K");
  });

  it("still offers a bare cap for a key named in prose", () => {
    render(<Kbd>Tab</Kbd>);
    expect(screen.getByText("Tab").tagName).toBe("KBD");
  });
});

describe("Label (P37)", () => {
  it("keeps the required note off the end of the label text", () => {
    // The note used to open with a space inside an `sr-only` span. The
    // accessible-name algorithm trims each node's contribution, so the space
    // was dropped and the field announced "Header row(required)".
    render(
      <Field label="Header row" required>
        <Input />
      </Field>,
    );
    const label = document.querySelector("label")!;
    expect(label.textContent).toContain(", required");
    expect(label.textContent).not.toMatch(/row\s*\(required\)/);
    expect(screen.getByRole("textbox")).toBeRequired();
  });
});

describe("List (P31)", () => {
  it("is a plain list, with no flag that paints a row as a control", () => {
    // `interactive` painted `.nav-item` on a bare `<li>`: a row that looked
    // like a control while staying unfocusable and announced as plain text.
    render(
      <List aria-label="Servers">
        <ListItem>Alpha</ListItem>
      </List>,
    );
    const item = within(screen.getByRole("list", { name: "Servers" })).getByRole("listitem");
    expect(item.className).toBe("");
    expect(item).not.toHaveAttribute("tabindex");

    // The flag is gone from the type; this pins that nothing answers to it at
    // runtime either, so a stale call site fails loudly instead of quietly
    // rendering the row it used to.
    cleanup();
    render(
      <List aria-label="Servers">
        {React.createElement(ListItem, { interactive: true } as Record<string, unknown>, "Alpha")}
      </List>,
    );
    const stale = within(screen.getByRole("list", { name: "Servers" })).getByRole("listitem");
    expect(stale.className).not.toContain("nav-item");
  });
});

describe("Heading roles (P36)", () => {
  it("keeps the look and the outline depth on separate props", () => {
    // A role used to be welded to one element, so a dialog that wanted
    // `SectionTitle`'s look at `<h2>` hand-wrote the recipe instead -- `Modal`
    // and `AlertDialog` each carried a copy that could drift from the day it
    // was written.
    render(
      <>
        <SectionTitle>Default depth</SectionTitle>
        <SectionTitle level={2}>Dialog title</SectionTitle>
      </>,
    );
    const usual = screen.getByText("Default depth");
    const raised = screen.getByText("Dialog title");
    expect(usual.tagName).toBe("H3");
    expect(raised.tagName).toBe("H2");
    // Same role, so the same recipe: only the outline depth moved.
    expect(raised.className).toBe(usual.className);
    expect(raised.className).toContain("type-heading");
    expect(raised.className).not.toMatch(/(?<![\w-])text-(?:xs|sm|base|lg|xl)(?![\w-])/);
  });

  it("gives a dialog title the role rather than a copy of it", () => {
    render(
      <Modal onClose={vi.fn()} open title="Export document">
        body
      </Modal>,
    );
    const title = screen.getByRole("heading", { name: "Export document" });
    expect(title.tagName).toBe("H2");
    expect(title.className).toContain("type-heading");
  });

  it("lets an eyebrow be the heading when the eyebrow IS the heading", () => {
    // A section labelled by its own eyebrow needs that eyebrow in the outline;
    // above hero copy it is a caption and stays a paragraph.
    const { rerender } = render(<Eyebrow>Saved</Eyebrow>);
    expect(screen.getByText("Saved").tagName).toBe("P");
    rerender(<Eyebrow level={2}>Saved</Eyebrow>);
    const asHeading = screen.getByRole("heading", { name: "Saved" });
    expect(asHeading.tagName).toBe("H2");
    expect(asHeading.className).toContain("type-eyebrow");
  });
});

describe("Chip (P39)", () => {
  it("answers the cursor when it is disabled", () => {
    render(<Chip disabled>Filter</Chip>);
    const chip = screen.getByRole("button", { name: "Filter" });
    expect(chip.className).toContain("disabled:cursor-not-allowed");
    expect(chip.className).not.toContain("disabled:cursor-default");
    // The token duration and curve, like every other transition in the app.
    expect(chip.className).toContain("duration-fast");
    expect(chip.className).toContain("ease-soft");
  });

  it("renders a static chip as text, with no control left on it", async () => {
    const onClick = vi.fn();
    render(
      <Chip interactive={false} onClick={onClick}>
        Read only
      </Chip>,
    );
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    // A span with a click handler is unreachable by keyboard. The prop type
    // refuses `disabled`, which a span silently drops.
    await userEvent.click(screen.getByText("Read only"));
    expect(onClick).toHaveBeenCalledOnce();
  });
});

describe("Pagination (P35)", () => {
  it("announces the position and keeps focus off a control it just disabled", async () => {
    const user = userEvent.setup();

    function PagerHarness() {
      const [page, setPage] = React.useState(1);
      return <Pagination onPageChange={setPage} page={page} pageCount={2} />;
    }

    render(<PagerHarness />);
    const status = screen.getByText(/Page/);
    expect(status).toHaveAttribute("aria-live", "polite");
    expect(status).toHaveTextContent("Page 1 of 2");

    const next = screen.getByRole("button", { name: "Next page" });
    const previous = screen.getByRole("button", { name: "Previous page" });
    expect(previous).toBeDisabled();

    await user.click(next);
    expect(screen.getByText(/Page/)).toHaveTextContent("Page 2 of 2");
    // `next` is disabled now, and a disabled element cannot hold focus -- the
    // browser would drop it on `<body>` and restart tabbing from the top.
    expect(next).toBeDisabled();
    expect(document.body).not.toHaveFocus();
    expect(previous).toHaveFocus();
  });
});

describe("the display primitives name themselves or say nothing", () => {
  it("keeps the mark silent unless it is given a name", () => {
    const { rerender } = render(<XFalconMark />);
    expect(document.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
    rerender(<XFalconMark label="xFalcon" />);
    expect(screen.getByRole("img", { name: "xFalcon" })).toBeInTheDocument();
  });

  it("gives a divider its orientation, since only one of the two is the default", () => {
    const { rerender } = render(<Divider orientation="vertical" />);
    expect(screen.getByRole("separator")).toHaveAttribute("aria-orientation", "vertical");
    rerender(<Divider />);
    expect(screen.getByRole("separator")).toHaveAttribute("aria-orientation", "horizontal");
  });

  it("takes a closed sidebar out of the keyboard order as well as out of the tree", () => {
    const { rerender } = render(
      <Sidebar aria-label="Workspace navigation" open={false}>
        <button type="button">Projects</button>
      </Sidebar>,
    );
    const aside = document.querySelector("aside")!;
    expect(aside).toHaveAttribute("aria-hidden", "true");
    // `inert` is what stops a hidden pane from still being tabbable.
    expect(aside).toHaveAttribute("inert");

    rerender(
      <Sidebar aria-label="Workspace navigation" open>
        <button type="button">Projects</button>
      </Sidebar>,
    );
    expect(screen.getByRole("complementary", { name: "Workspace navigation" })).toBeInTheDocument();
    expect(document.querySelector("aside")).not.toHaveAttribute("inert");
  });

  it("leaves badge and avatar as text, with no invented role", () => {
    // Neither is a control and neither is an image: whatever they contain is
    // the whole of what they say. A role here would announce a shape.
    render(
      <>
        <Badge tone="warn">3 warnings</Badge>
        <Avatar size="sm">JT</Avatar>
      </>,
    );
    expect(screen.getByText("3 warnings").tagName).toBe("SPAN");
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("keeps the top chrome a banner landmark in both variants", () => {
    const { rerender } = render(<NavBar>Title</NavBar>);
    expect(screen.getByRole("banner")).toHaveClass("app-titlebar");
    rerender(<NavBar variant="plain">Title</NavBar>);
    expect(screen.getByRole("banner")).not.toHaveClass("app-titlebar");
  });
});
