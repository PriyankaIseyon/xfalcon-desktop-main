import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Select, type SelectOption } from "./Select";

afterEach(cleanup);

const models: SelectOption[] = [
  { value: "sonnet-openrouter", label: "Sonnet", group: "OpenRouter" },
  { value: "grok", label: "Grok", group: "OpenRouter" },
  { value: "opus", label: "Opus", group: "Claude account" },
  { value: "haiku", label: "Haiku", group: "Claude account" },
];

/** The menu's rows in DOM order: a heading's text, an option's label, or `---` for a separator. */
function rowsOf(menu: HTMLElement): string[] {
  return Array.from(menu.children).map((row) => {
    if (row.getAttribute("role") === "separator") return "---";
    if (row.getAttribute("role") === "menuitemradio") return `option:${row.textContent}`;
    return `heading:${row.textContent}`;
  });
}

describe("Select groups", () => {
  it("headings each run of options once and sets runs apart with a separator", async () => {
    const user = userEvent.setup();
    render(<Select onChange={vi.fn()} options={models} value="sonnet-openrouter" />);

    await user.click(screen.getByRole("button"));

    expect(rowsOf(screen.getByRole("menu"))).toEqual([
      "heading:OpenRouter",
      "option:Sonnet",
      "option:Grok",
      "---",
      "heading:Claude account",
      "option:Opus",
      "option:Haiku",
    ]);
  });

  it("renders no heading and no separator when no option has a group", async () => {
    const user = userEvent.setup();
    render(
      <Select
        onChange={vi.fn()}
        options={[
          { value: "alpha", label: "Alpha" },
          { value: "beta", label: "Beta" },
        ]}
        value="alpha"
      />,
    );

    await user.click(screen.getByRole("button"));

    expect(rowsOf(screen.getByRole("menu"))).toEqual(["option:Alpha", "option:Beta"]);
  });

  it("still marks the selected option under a heading", async () => {
    const user = userEvent.setup();
    render(<Select onChange={vi.fn()} options={models} value="opus" />);

    await user.click(screen.getByRole("button"));

    expect(screen.getByRole("menuitemradio", { name: "Opus" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getAllByRole("menuitemradio", { checked: true })).toHaveLength(1);
  });

  it("moves arrow-key focus from one option to the next across a heading and separator", async () => {
    const user = userEvent.setup();
    render(<Select onChange={vi.fn()} options={models} value="grok" />);

    await user.click(screen.getByRole("button"));
    expect(screen.getByRole("menuitemradio", { name: "Grok" })).toHaveFocus();

    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("menuitemradio", { name: "Opus" })).toHaveFocus();

    await user.keyboard("{ArrowUp}");
    expect(screen.getByRole("menuitemradio", { name: "Grok" })).toHaveFocus();
  });
});
