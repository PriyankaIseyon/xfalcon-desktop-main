import React from "react";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Plus, Trash2 } from "lucide-react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SettingsButton, SettingsIconButton, SettingsSegmentedControl } from "./SettingsControls";

// Without this each case tabs into the previous case's buttons.
afterEach(cleanup);

describe("Settings controls", () => {
  it("renders a native action button with the Settings family API", () => {
    render(<SettingsButton>Check for updates</SettingsButton>);

    const button = screen.getByRole("button", { name: "Check for updates" });
    expect(button).toHaveAttribute("type", "button");
    expect(button).toBeEnabled();
  });

  it("keeps action icons decorative so the visible label supplies the native name", () => {
    render(<SettingsButton icon={<Plus aria-label="Plus" />}>Add server</SettingsButton>);

    expect(screen.getByRole("button", { name: "Add server" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Plus Add server" })).not.toBeInTheDocument();
  });

  it("keeps the original label visible and disables a loading action", () => {
    render(<SettingsButton loading>Save provider</SettingsButton>);

    const button = screen.getByRole("button", { name: "Save provider" });
    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("aria-busy", "true");
    expect(button).toHaveTextContent("Save provider");
  });

  it("names an icon-only action and shows that name on focus", async () => {
    const user = userEvent.setup();
    render(<SettingsIconButton aria-label="Remove provider" icon={<Trash2 />} variant="danger" />);

    const button = screen.getByRole("button", { name: "Remove provider" });
    expect(button).toHaveAttribute("type", "button");
    // No native title: the hint is the product's own tooltip, which a keyboard
    // reaches and a native one does not.
    expect(button).not.toHaveAttribute("title");

    await user.tab();
    expect(button).toHaveFocus();
    expect(screen.getByRole("tooltip")).toHaveTextContent("Remove provider");
  });

  it("composes a named Settings radiogroup with native radio semantics", () => {
    render(
      <SettingsSegmentedControl
        aria-label="Theme"
        options={[
          { label: "System", value: "system" },
          { label: "Light", value: "light" },
        ]}
        value="system"
        onChange={vi.fn()}
      />,
    );

    const group = screen.getByRole("radiogroup", { name: "Theme" });
    expect(group).toHaveAttribute("data-settings-segmented-control");
    expect(within(group).getByRole("radio", { name: "System" })).toHaveAttribute("aria-checked", "true");
    expect(within(group).getByRole("radio", { name: "Light" })).toHaveAttribute("aria-checked", "false");
  });

  it("accepts aria-labelledby as its required radiogroup name", () => {
    render(
      <>
        <span id="reading-size-label">Reading size</span>
        <SettingsSegmentedControl
          aria-labelledby="reading-size-label"
          options={[
            { label: "Small", value: "small" },
            { label: "Large", value: "large" },
          ]}
          value="small"
          onChange={vi.fn()}
        />
      </>,
    );

    expect(screen.getByRole("radiogroup", { name: "Reading size" })).toBeInTheDocument();
  });

  it("moves focus with selection and wraps in both directions", async () => {
    const user = userEvent.setup();

    function SettingsSegmentedHarness() {
      const [value, setValue] = React.useState("first");
      return (
        <SettingsSegmentedControl
          aria-label="Settings view"
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

    render(<SettingsSegmentedHarness />);
    const group = screen.getByRole("radiogroup", { name: "Settings view" });
    const first = within(group).getByRole("radio", { name: "First" });
    const third = within(group).getByRole("radio", { name: "Third" });
    first.focus();

    await user.keyboard("{ArrowLeft}");
    expect(third).toHaveAttribute("aria-checked", "true");
    expect(third).toHaveAttribute("tabindex", "0");
    expect(third).toHaveFocus();

    await user.keyboard("{ArrowRight}");
    expect(first).toHaveAttribute("aria-checked", "true");
    expect(first).toHaveAttribute("tabindex", "0");
    expect(first).toHaveFocus();
  });
});
