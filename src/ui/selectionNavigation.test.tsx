import React from "react";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Accordion } from "./Accordion";
import { Checkbox } from "./Checkbox";
import { ContextMenu } from "./ContextMenu";
import { Menu } from "./Menu";
import { Modal } from "./Modal";
import { Radio, RadioGroup } from "./Radio";
import { SegmentedControl } from "./SegmentedControl";
import { Slider } from "./Slider";
import { Switch } from "./Switch";
import { Tabs } from "./Tabs";

afterEach(cleanup);

describe("selection controls", () => {
  it("keeps the checkbox itself visible to the shared focus ring and exposes mixed state", () => {
    render(<Checkbox aria-label="Select all" indeterminate />);

    const checkbox = screen.getByRole("checkbox", { name: "Select all" });
    expect(checkbox).toBePartiallyChecked();
    expect(checkbox).not.toHaveClass("sr-only");
    expect(checkbox).toHaveClass("rounded-sm");
    expect(checkbox.parentElement?.innerHTML).not.toMatch(/peer-focus-visible:(?:ring|outline)/);
  });

  it("moves radio focus and selection as one native group", async () => {
    const user = userEvent.setup();

    function RadioHarness() {
      const [value, setValue] = React.useState("alpha");
      return (
        <RadioGroup label="Mode" name="mode" value={value} onValueChange={setValue}>
          <Radio value="alpha">Alpha</Radio>
          <Radio value="beta">Beta</Radio>
        </RadioGroup>
      );
    }

    render(<RadioHarness />);
    const alpha = screen.getByRole("radio", { name: "Alpha" });
    const beta = screen.getByRole("radio", { name: "Beta" });
    alpha.focus();
    await user.keyboard("{ArrowDown}");

    expect(beta).toHaveFocus();
    expect(beta).toBeChecked();
    expect(beta).not.toHaveClass("sr-only");
  });

  it("makes a busy switch truthful and prevents repeat activation", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Switch busy checked={false} label="Sync" onChange={onChange} />);

    const control = screen.getByRole("switch", { name: "Sync" });
    expect(control).toHaveAttribute("aria-busy", "true");
    expect(control).toBeDisabled();
    expect(control.className).not.toMatch(/focus:(?:outline|ring)-none/);
    await user.click(control);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("lets the native slider own its label, value, and keyboard step", async () => {
    render(<Slider defaultValue={4} label="Detail" max={10} min={0} step={2} />);

    const slider = screen.getByRole("slider", { name: "Detail" });
    expect(slider).toHaveValue("4");
    expect(slider).toHaveAttribute("min", "0");
    expect(slider).toHaveAttribute("max", "10");
    expect(slider).toHaveAttribute("step", "2");
    expect(slider.className).not.toMatch(/focus:(?:outline|ring)-none/);
  });
});

describe("segmented controls and tabs", () => {
  it.each(["solid", "settings"] as const)(
    "moves focus with automatic %s segment selection and skips disabled choices",
    async (variant) => {
      const user = userEvent.setup();

      function SegmentHarness() {
        const [value, setValue] = React.useState("first");
        return (
          <SegmentedControl
            aria-label={`${variant} view`}
            onChange={setValue}
            options={[
              { label: "First", value: "first" },
              { disabled: true, label: "Second", value: "second" },
              { label: "Third", value: "third" },
            ]}
            value={value}
            variant={variant}
          />
        );
      }

      render(<SegmentHarness />);
      const group = screen.getByRole("radiogroup", { name: `${variant} view` });
      const first = within(group).getByRole("radio", { name: "First" });
      const second = within(group).getByRole("radio", { name: "Second" });
      const third = within(group).getByRole("radio", { name: "Third" });
      first.focus();
      await user.keyboard("{ArrowRight}");

      expect(second).toBeDisabled();
      expect(third).toHaveFocus();
      expect(third).toHaveAttribute("aria-checked", "true");

      await user.keyboard("{Home}");
      expect(first).toHaveFocus();
      expect(first).toHaveAttribute("aria-checked", "true");
    },
  );

  it("wires each tab to the one panel it selects and supports Home and End", async () => {
    const user = userEvent.setup();

    function TabsHarness() {
      const [value, setValue] = React.useState("file");
      return (
        <Tabs
          aria-label="File sections"
          onChange={setValue}
          tabs={[
            { id: "file", label: "File" },
            { id: "history", label: "History" },
          ]}
          value={value}
        >
          <p>{value === "file" ? "File body" : "History body"}</p>
        </Tabs>
      );
    }

    render(<TabsHarness />);
    const tablist = screen.getByRole("tablist", { name: "File sections" });
    const file = within(tablist).getByRole("tab", { name: "File" });
    const history = within(tablist).getByRole("tab", { name: "History" });
    const filePanel = screen.getByRole("tabpanel", { name: "File" });
    expect(file).toHaveAttribute("aria-controls", filePanel.id);
    expect(filePanel).toHaveAttribute("aria-labelledby", file.id);
    // Only one panel is ever in the document, so only the selected tab
    // controls anything. A per-tab id gave every other tab an `aria-controls`
    // pointing at no element at all.
    expect(history).not.toHaveAttribute("aria-controls");
    for (const tab of within(tablist).getAllByRole("tab")) {
      const controls = tab.getAttribute("aria-controls");
      if (controls != null) expect(document.getElementById(controls)).not.toBeNull();
    }

    file.focus();
    await user.keyboard("{End}");
    expect(history).toHaveFocus();
    expect(screen.getByRole("tabpanel", { name: "History" })).toHaveTextContent("History body");
    await user.keyboard("{Home}");
    expect(file).toHaveFocus();
  });
});

describe("disclosure and menu families", () => {
  it("keeps accordion relationships unique and responds to controlled state", () => {
    const item = { id: "details", title: "Details", children: "The details", open: false };
    const first = render(<Accordion items={[item]} />);
    const firstTrigger = screen.getByRole("button", { name: "Details" });
    expect(firstTrigger).toHaveAttribute("aria-expanded", "false");

    first.rerender(<Accordion items={[{ ...item, open: true }]} />);
    const firstPanel = screen.getByRole("region", { name: "Details" });
    expect(firstTrigger).toHaveAttribute("aria-controls", firstPanel.id);
    expect(firstPanel).toHaveAttribute("aria-labelledby", firstTrigger.id);

    render(<Accordion items={[{ ...item, open: true }]} />);
    const triggers = screen.getAllByRole("button", { name: "Details" });
    expect(triggers[0].id).not.toBe(triggers[1].id);
    expect(triggers[0].getAttribute("aria-controls")).not.toBe(triggers[1].getAttribute("aria-controls"));
  });

  it("opens a trigger menu with ArrowDown, links it to the trigger, and restores focus on Escape", async () => {
    const user = userEvent.setup();
    render(
      <Menu trigger={<button type="button">Actions</button>}>
        <Menu.Item disabled>Unavailable</Menu.Item>
        <Menu.Item>Rename</Menu.Item>
        <Menu.Item>Delete</Menu.Item>
      </Menu>,
    );

    const trigger = screen.getByRole("button", { name: "Actions" });
    trigger.focus();
    await user.keyboard("{ArrowDown}");
    const menu = screen.getByRole("menu", { name: "Actions" });
    expect(trigger).toHaveAttribute("aria-controls", menu.id);
    expect(menu).toHaveAttribute("aria-labelledby", trigger.id);
    expect(screen.getByRole("menuitem", { name: "Rename" })).toHaveFocus();

    await user.keyboard("d");
    expect(screen.getByRole("menuitem", { name: "Delete" })).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });

  it("states menu selection and danger without caller-owned styling", async () => {
    const user = userEvent.setup();
    render(
      <Menu trigger={<button type="button">Choose</button>}>
        <Menu.Item selected>Current</Menu.Item>
        <Menu.Item selected={false}>Other</Menu.Item>
        <Menu.Item variant="danger">Delete</Menu.Item>
      </Menu>,
    );

    await user.click(screen.getByRole("button", { name: "Choose" }));
    expect(screen.getByRole("menuitemradio", { name: "Current" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("menuitemradio", { name: "Other" })).toHaveAttribute("aria-checked", "false");
    expect(screen.getByRole("menuitem", { name: "Delete" })).toHaveAttribute("data-variant", "danger");
  });

  it("flips and clamps a trigger menu at the bottom-right viewport edge", async () => {
    const user = userEvent.setup();
    const width = vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(120);
    const height = vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(100);
    render(
      <Menu align="end" trigger={<button type="button">Edge actions</button>}>
        <Menu.Item>Rename</Menu.Item>
      </Menu>,
    );

    const trigger = screen.getByRole("button", { name: "Edge actions" });
    vi.spyOn(trigger, "getBoundingClientRect").mockReturnValue({
      bottom: window.innerHeight - 4,
      height: 36,
      left: window.innerWidth - 48,
      right: window.innerWidth - 8,
      top: window.innerHeight - 40,
      width: 40,
      x: window.innerWidth - 48,
      y: window.innerHeight - 40,
      toJSON: () => ({}),
    });

    await user.click(trigger);
    const menu = screen.getByRole("menu", { name: "Edge actions" });
    expect(Number.parseFloat(menu.style.left)).toBeLessThanOrEqual(window.innerWidth - 128);
    expect(Number.parseFloat(menu.style.top)).toBe(window.innerHeight - 144);
    // The width the placement was solved for, so the panel cannot re-lay-out
    // narrower than the edge it was aligned to. The mocked panel measures 120
    // against a 40 px trigger, so the menu holds 120.
    expect(menu.style.minWidth).toBe("120px");
    width.mockRestore();
    height.mockRestore();
  });

  it("keeps a portalled menu inside the modal focus scope that opened it", async () => {
    const user = userEvent.setup();
    render(
      <Modal onClose={vi.fn()} open title="Choose a format">
        <Menu trigger={<button type="button">Format</button>}>
          <Menu.Item>PDF</Menu.Item>
        </Menu>
      </Modal>,
    );

    await user.click(screen.getByRole("button", { name: "Format" }));
    const dialog = screen.getByRole("dialog", { name: "Choose a format" });
    expect(dialog).toContainElement(screen.getByRole("menu", { name: "Format" }));
  });

  it("closes only the menu when Escape is pressed inside a dialog", async () => {
    // Both the modal trap and the menu used to keep their own `document`
    // keydown listener in the capture phase, and listeners on one node in one
    // phase run in REGISTRATION order -- so the modal, which always registers
    // first, answered Escape and took the dialog with it. `dismissLayers`
    // gives the keystroke to the layer opened last instead.
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(
      <Modal onClose={onClose} open title="Choose a format">
        <Menu trigger={<button type="button">Format</button>}>
          <Menu.Item>PDF</Menu.Item>
        </Menu>
      </Modal>,
    );

    const trigger = screen.getByRole("button", { name: "Format" });
    await user.click(trigger);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
    expect(trigger).toHaveFocus();
    expect(screen.getByRole("dialog", { name: "Choose a format" })).toBeInTheDocument();

    // The dialog is the top layer again, so the next Escape is its own.
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("hands focus back to the trigger when Tab leaves an open menu", async () => {
    // Tab used to close the menu without returning focus, so the focused item
    // unmounted under the caret, leaving `<body>` focused -- and the browser,
    // which reads focus AFTER dispatch to pick the next stop, then resumed
    // tabbing from the top of the document.
    //
    // Asserted with a raw Tab rather than `user.tab()` on purpose: user-event
    // picks its destination BEFORE dispatching the key, so it cannot observe a
    // handler that moves focus, and would report the emulator's answer instead
    // of the app's. What this primitive owes is exactly the state the browser
    // reads next: menu closed, trigger focused.
    const user = userEvent.setup();
    render(
      <>
        <Menu trigger={<button type="button">Actions</button>}>
          <Menu.Item>Rename</Menu.Item>
        </Menu>
        <button type="button">After</button>
      </>,
    );

    const trigger = screen.getByRole("button", { name: "Actions" });
    await user.click(trigger);
    const item = screen.getByRole("menuitem", { name: "Rename" });
    expect(item).toHaveFocus();

    fireEvent.keyDown(item, { key: "Tab" });
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });

  it("clamps a context menu and returns focus to the row that opened it", async () => {
    const user = userEvent.setup();
    const width = vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(120);
    const height = vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(100);
    const row = document.createElement("button");
    row.textContent = "Quarterly model";
    document.body.append(row);
    row.focus();
    const onClose = vi.fn();

    render(
      <ContextMenu anchor={{ x: 999, y: 999, returnFocus: row }} label="Row actions" onClose={onClose}>
        <Menu.Item>Rename</Menu.Item>
      </ContextMenu>,
    );

    const menu = await screen.findByRole("menu", { name: "Row actions" });
    expect(Number.parseFloat(menu.style.left)).toBeLessThanOrEqual(window.innerWidth - 128);
    expect(Number.parseFloat(menu.style.top)).toBeLessThanOrEqual(window.innerHeight - 108);
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(row).toHaveFocus();
    row.remove();
    width.mockRestore();
    height.mockRestore();
  });
});

it("keeps a menu open when its own results scroll and dismisses on outside scroll", async () => {
  const user = userEvent.setup();
  render(
    <Menu trigger={<button>Scroll actions</button>}>
      <Menu.Item>First</Menu.Item>
      <Menu.Item>Last</Menu.Item>
    </Menu>,
  );
  await user.click(screen.getByRole("button", { name: "Scroll actions" }));
  fireEvent.scroll(screen.getByRole("menu"));
  expect(screen.getByRole("menu")).toBeVisible();
  fireEvent.scroll(window);
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
});
