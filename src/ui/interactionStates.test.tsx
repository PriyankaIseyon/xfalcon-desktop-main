import React from "react";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Button } from "./Button";
import { Checkbox } from "./Checkbox";
import { Chip } from "./Chip";
import { IconButton } from "./IconButton";
import { Radio, RadioGroup } from "./Radio";
import { SegmentedControl } from "./SegmentedControl";
import { Switch } from "./Switch";

afterEach(cleanup);

/**
 * The shared interaction-state grammar, asserted once for every primitive it
 * applies to (spec: Decision "Complete interaction grammar").
 *
 * Two rules carry most of the weight, because both had been left to each
 * primitive to remember:
 *
 * - A control that reports itself disabled must also refuse to act. An
 *   `opacity-50` that still fires `onClick` is a lie the user acts on.
 * - A control that reports itself busy must be disabled too, for the same
 *   reason: `aria-busy` says "working", and a second click during that window
 *   submits twice.
 */
type Selection = "pressed" | "checked" | "native";

type Primitive = {
  name: string;
  role: string;
  accessibleName: string;
  /** Rest, with a click handler so activation can be observed. */
  render: (onClick: () => void) => React.ReactElement;
  renderDisabled: (onClick: () => void) => React.ReactElement;
  /** Primitives that report work in flight. */
  renderBusy?: (onClick: () => void) => React.ReactElement;
  /** How this primitive says it is on, and how it says it is off. */
  selection?: { kind: Selection; renderOn: () => React.ReactElement; renderOff: () => React.ReactElement };
  /** Whether the primitive carries the shared press affordance. */
  pressAffordance: boolean;
};

const primitives: Primitive[] = [
  {
    name: "Button",
    role: "button",
    accessibleName: "Save",
    render: (onClick) => <Button onClick={onClick}>Save</Button>,
    renderDisabled: (onClick) => (
      <Button disabled onClick={onClick}>
        Save
      </Button>
    ),
    renderBusy: (onClick) => (
      <Button loading onClick={onClick}>
        Save
      </Button>
    ),
    selection: {
      kind: "pressed",
      renderOn: () => <Button selected>Save</Button>,
      renderOff: () => <Button selected={false}>Save</Button>,
    },
    pressAffordance: true,
  },
  {
    name: "IconButton",
    role: "button",
    accessibleName: "Delete",
    render: (onClick) => (
      <IconButton aria-label="Delete" onClick={onClick}>
        x
      </IconButton>
    ),
    renderDisabled: (onClick) => (
      <IconButton aria-label="Delete" disabled onClick={onClick}>
        x
      </IconButton>
    ),
    renderBusy: (onClick) => (
      <IconButton aria-label="Delete" loading onClick={onClick}>
        x
      </IconButton>
    ),
    selection: {
      kind: "pressed",
      renderOn: () => (
        <IconButton aria-label="Delete" selected>
          x
        </IconButton>
      ),
      renderOff: () => (
        <IconButton aria-label="Delete" selected={false}>
          x
        </IconButton>
      ),
    },
    pressAffordance: true,
  },
  {
    name: "Chip",
    role: "button",
    accessibleName: "Filter",
    render: (onClick) => <Chip onClick={onClick}>Filter</Chip>,
    renderDisabled: (onClick) => (
      <Chip disabled onClick={onClick}>
        Filter
      </Chip>
    ),
    pressAffordance: true,
  },
  {
    name: "Checkbox",
    role: "checkbox",
    accessibleName: "Accept",
    render: (onClick) => <Checkbox aria-label="Accept" onClick={onClick} />,
    renderDisabled: (onClick) => <Checkbox aria-label="Accept" disabled onClick={onClick} />,
    selection: {
      kind: "native",
      renderOn: () => <Checkbox aria-label="Accept" checked readOnly />,
      renderOff: () => <Checkbox aria-label="Accept" checked={false} readOnly />,
    },
    pressAffordance: false,
  },
  {
    name: "Switch",
    role: "switch",
    accessibleName: "Reasoning",
    render: (onClick) => <Switch checked={false} label="Reasoning" onChange={onClick} />,
    renderDisabled: (onClick) => <Switch checked={false} disabled label="Reasoning" onChange={onClick} />,
    renderBusy: (onClick) => <Switch busy checked={false} label="Reasoning" onChange={onClick} />,
    selection: {
      kind: "checked",
      renderOn: () => <Switch checked label="Reasoning" onChange={() => {}} />,
      renderOff: () => <Switch checked={false} label="Reasoning" onChange={() => {}} />,
    },
    pressAffordance: true,
  },
];

const SELECTION_ATTRIBUTE: Record<Selection, string> = {
  pressed: "aria-pressed",
  checked: "aria-checked",
  native: "checked",
};

describe.each(primitives)("$name interaction states", (primitive) => {
  it("rests as an operable control with its own name", async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(primitive.render(onClick));

    const control = screen.getByRole(primitive.role, { name: primitive.accessibleName });
    expect(control).toBeEnabled();
    expect(control).not.toHaveAttribute("aria-busy", "true");

    await user.click(control);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("refuses to act while it reports itself disabled", async () => {
    const user = userEvent.setup();
    const onClick = vi.fn();
    render(primitive.renderDisabled(onClick));

    const control = screen.getByRole(primitive.role, { name: primitive.accessibleName });
    expect(control).toBeDisabled();

    await user.click(control);
    expect(onClick).not.toHaveBeenCalled();

    // Nor by keyboard: a disabled control is out of the tab order.
    await user.tab();
    expect(control).not.toHaveFocus();
  });

  it("takes keyboard focus when it is operable", async () => {
    const user = userEvent.setup();
    render(primitive.render(vi.fn()));

    await user.tab();
    expect(screen.getByRole(primitive.role, { name: primitive.accessibleName })).toHaveFocus();
  });

  it("declares no focus treatment of its own", () => {
    render(primitive.render(vi.fn()));
    const control = screen.getByRole(primitive.role, { name: primitive.accessibleName });
    // One ring, declared once in `index.css` for every focusable element.
    expect(control.className).not.toMatch(/focus-visible:(ring|outline-none)/);
  });

  it("carries the shared press affordance where pressing it should answer", () => {
    render(primitive.render(vi.fn()));
    const control = screen.getByRole(primitive.role, { name: primitive.accessibleName });
    const carriesPress = control.className.split(/\s+/).includes("press");
    expect(carriesPress || control.closest(".press") !== null).toBe(primitive.pressAffordance);
  });

  const renderBusy = primitive.renderBusy;
  if (renderBusy) {
    it("stops answering while it reports work in flight", async () => {
      const user = userEvent.setup();
      const onClick = vi.fn();
      render(renderBusy(onClick));

      const control = screen.getByRole(primitive.role, { name: primitive.accessibleName });
      expect(control).toHaveAttribute("aria-busy", "true");
      // Busy without disabled submits twice on a double click.
      expect(control).toBeDisabled();

      await user.click(control);
      expect(onClick).not.toHaveBeenCalled();
    });
  }

  if (primitive.selection) {
    const { kind, renderOn, renderOff } = primitive.selection;
    const find = () => screen.getByRole(primitive.role, { name: primitive.accessibleName });

    it(`says it is on with ${SELECTION_ATTRIBUTE[kind]}`, () => {
      render(renderOn());
      if (kind === "native") expect(find()).toBeChecked();
      else expect(find()).toHaveAttribute(SELECTION_ATTRIBUTE[kind], "true");
    });

    it(`says it is off with ${SELECTION_ATTRIBUTE[kind]}, not by omitting it`, () => {
      render(renderOff());
      // A missing attribute reads as "not a toggle", which is a different claim
      // from "a toggle that is off".
      if (kind === "native") expect(find()).not.toBeChecked();
      else expect(find()).toHaveAttribute(SELECTION_ATTRIBUTE[kind], "false");
    });
  }
});

describe("selection groups", () => {
  it("gives a radio group one checked member and roving focus", async () => {
    const user = userEvent.setup();
    const onValueChange = vi.fn();
    render(
      <RadioGroup label="Mode" name="mode" value="a" onValueChange={onValueChange}>
        <Radio value="a">A</Radio>
        <Radio value="b">B</Radio>
      </RadioGroup>,
    );

    expect(screen.getByRole("radio", { name: "A" })).toBeChecked();
    expect(screen.getByRole("radio", { name: "B" })).not.toBeChecked();

    await user.click(screen.getByRole("radio", { name: "B" }));
    expect(onValueChange).toHaveBeenCalledWith("b");
  });

  it("marks exactly one segment selected", () => {
    render(
      <SegmentedControl
        aria-label="Theme"
        options={[
          { value: "light", label: "Light" },
          { value: "dark", label: "Dark" },
        ]}
        value="dark"
        onChange={vi.fn()}
      />,
    );

    const selected = screen.getAllByRole("radio").filter((option) => option.getAttribute("aria-checked") === "true");
    expect(selected).toHaveLength(1);
    expect(selected[0]).toHaveAccessibleName("Dark");
  });
});
