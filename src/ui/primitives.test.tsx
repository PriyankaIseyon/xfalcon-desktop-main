import React from "react";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Alert } from "./Alert";
import { Checkbox } from "./Checkbox";
import { Progress } from "./Progress";
import { Radio, RadioGroup } from "./Radio";
import { SegmentedControl } from "./SegmentedControl";

describe("design system primitives", () => {
  it("exposes alert semantics by tone", () => {
    render(<Alert tone="danger">Something failed</Alert>);
    expect(screen.getByRole("alert")).toHaveTextContent("Something failed");
  });

  it("toggles checkbox with keyboard", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Checkbox aria-label="Accept" onChange={onChange} />);
    const box = screen.getByRole("checkbox", { name: "Accept" });
    await user.click(box);
    expect(onChange).toHaveBeenCalled();
  });

  it("moves radio selection inside a group", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(
      <RadioGroup label="Mode" name="mode" value="a" onValueChange={onChange}>
        <Radio value="a">Alpha</Radio>
        <Radio value="b">Beta</Radio>
      </RadioGroup>,
    );
    await user.click(screen.getByRole("radio", { name: "Beta" }));
    expect(onChange).toHaveBeenCalledWith("b");
  });

  it("reports progress values to assistive tech", () => {
    render(<Progress label="Upload" max={100} value={40} />);
    expect(screen.getByRole("progressbar", { name: "Upload" })).toHaveAttribute("aria-valuenow", "40");
  });

  it("names a segmented radiogroup and keeps the default outside the Settings variant", () => {
    render(
      <SegmentedControl
        aria-label="Density"
        options={[
          { label: "Compact", value: "compact" },
          { label: "Comfortable", value: "comfortable" },
        ]}
        value="compact"
        onChange={vi.fn()}
      />,
    );

    const group = screen.getByRole("radiogroup", { name: "Density" });
    expect(group).not.toHaveAttribute("data-settings-segmented-control");
  });

  it("keeps solid focus behavior while wrapping selection and roving tabindex", async () => {
    const user = userEvent.setup();

    function SegmentedHarness() {
      const [value, setValue] = React.useState("first");
      return (
        <SegmentedControl
          aria-label="View solid"
          options={[
            { label: "First", value: "first" },
            { label: "Second", value: "second" },
            { label: "Third", value: "third" },
          ]}
          value={value}
          onChange={setValue}
        />
      );
    }

    render(<SegmentedHarness />);
    const group = screen.getByRole("radiogroup", { name: "View solid" });
    const first = within(group).getByRole("radio", { name: "First" });
    const second = within(group).getByRole("radio", { name: "Second" });
    const third = within(group).getByRole("radio", { name: "Third" });

    expect(first).toHaveAttribute("tabindex", "0");
    expect(first).toHaveAttribute("aria-checked", "true");
    expect(second).toHaveAttribute("tabindex", "-1");
    first.focus();

    await user.keyboard("{ArrowLeft}");
    expect(third).toHaveAttribute("tabindex", "0");
    expect(third).toHaveAttribute("aria-checked", "true");
    expect(first).toHaveAttribute("tabindex", "-1");
    expect(third).toHaveFocus();

    await user.keyboard("{ArrowRight}");
    expect(first).toHaveAttribute("tabindex", "0");
    expect(first).toHaveAttribute("aria-checked", "true");
    expect(first).toHaveFocus();
  });
});
