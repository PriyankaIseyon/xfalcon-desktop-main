import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  SettingsActions,
  SettingsGroup,
  SettingsGroupHeader,
  SettingsPage,
  SettingsRow,
  SettingsSection,
} from "./SettingsLayout";

describe("Settings layout primitives", () => {
  it("provides a centered page content hook without adding a second main landmark", () => {
    const { container } = render(<SettingsPage>Page content</SettingsPage>);

    expect(container.querySelector("[data-settings-page]")).toHaveTextContent("Page content");
    expect(container.querySelector("[data-settings-page-content]")).toHaveTextContent("Page content");
    expect(screen.queryByRole("main")).not.toBeInTheDocument();
  });

  it("names each section with its rendered heading", () => {
    render(
      <SettingsSection title="Appearance" description="Choose how xFalcon looks.">
        Section content
      </SettingsSection>,
    );

    const section = screen.getByRole("region", { name: "Appearance" });
    expect(within(section).getByRole("heading", { name: "Appearance", level: 3 })).toBeInTheDocument();
    expect(within(section).getByText("Choose how xFalcon looks.")).toBeInTheDocument();
  });

  it("exposes one grouped surface for related settings", () => {
    render(<SettingsGroup aria-label="Appearance preferences">Preferences</SettingsGroup>);

    expect(screen.getByRole("group", { name: "Appearance preferences" })).toHaveAttribute("data-settings-group");
  });

  it("draws the box only on the outer group when groups are nested", () => {
    const { container } = render(
      <SettingsGroup aria-label="Outer block">
        <SettingsGroup id="inner-block" aria-label="Inner block">
          Inner content
        </SettingsGroup>
      </SettingsGroup>,
    );

    const outer = screen.getByRole("group", { name: "Outer block" });
    const inner = screen.getByRole("group", { name: "Inner block" });
    expect(outer).not.toHaveClass("settings-group--nested");
    expect(outer).not.toHaveAttribute("data-settings-group-nested");
    expect(inner).toHaveClass("settings-group", "settings-group--nested");
    expect(inner).toHaveAttribute("data-settings-group-nested");
    expect(inner).toHaveAttribute("id", "inner-block");
    expect(container.querySelectorAll(".settings-group--nested")).toHaveLength(1);
  });

  it("keeps a group standing alone as the only box, even after a nested sibling", () => {
    render(
      <>
        <SettingsGroup aria-label="Standalone">Standalone content</SettingsGroup>
        <SettingsGroup aria-label="Second standalone">Second content</SettingsGroup>
      </>,
    );

    expect(screen.getByRole("group", { name: "Standalone" })).not.toHaveClass("settings-group--nested");
    expect(screen.getByRole("group", { name: "Second standalone" })).not.toHaveClass("settings-group--nested");
  });

  it("aligns group header copy before its optional trailing action", () => {
    const { container } = render(
      <SettingsGroupHeader title="Model providers" description="Keys stored on this device">
        <button type="button">Add provider</button>
      </SettingsGroupHeader>,
    );

    const header = container.querySelector<HTMLElement>("[data-settings-group-header]");
    const copy = header?.querySelector<HTMLElement>("[data-settings-group-header-copy]");
    const actions = header?.querySelector<HTMLElement>("[data-settings-actions]");
    if (!header || !copy || !actions) throw new Error("SettingsGroupHeader structure is incomplete");
    expect(within(copy).getByRole("heading", { name: "Model providers", level: 4 })).toBeInTheDocument();
    expect(within(actions).getByRole("button", { name: "Add provider" })).toBeInTheDocument();
    expect(copy.compareDocumentPosition(actions) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("renders one leading copy region followed by one trailing control region", () => {
    const { container } = render(
      <SettingsRow label="Theme" hint="Choose a color appearance.">
        <button type="button">System</button>
      </SettingsRow>,
    );

    const row = container.querySelector<HTMLElement>("[data-settings-row]");
    const copy = row?.querySelector<HTMLElement>("[data-settings-row-copy]");
    const control = row?.querySelector<HTMLElement>("[data-settings-row-control]");
    if (!row || !copy || !control) throw new Error("SettingsRow structure is incomplete");
    expect(within(copy).getByText("Theme")).toBeInTheDocument();
    expect(within(copy).getByText("Choose a color appearance.")).toBeInTheDocument();
    expect(within(control).getByRole("button", { name: "System" })).toBeInTheDocument();
    expect(copy.nextElementSibling).toBe(control);
  });

  it("provides the common action alignment region", () => {
    const { container } = render(
      <SettingsActions>
        <button type="button">Cancel</button>
        <button type="button">Save</button>
      </SettingsActions>,
    );

    const actions = container.querySelector<HTMLElement>("[data-settings-actions]");
    expect(actions).not.toBeNull();
    if (!actions) throw new Error("SettingsActions did not render its stable hook");
    expect(within(actions).getAllByRole("button")).toHaveLength(2);
    expect(actions).toHaveAttribute("data-settings-actions");
  });
});
