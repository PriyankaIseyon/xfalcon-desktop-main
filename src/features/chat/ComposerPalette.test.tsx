import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Composer } from "./Composer";
import { ComposerPalette } from "./ComposerPalette";
import { DEFAULT_SLASH_COMMANDS } from "./composerDefaults";

afterEach(cleanup);

describe("ComposerPalette", () => {
  it("renders slash command items and calls onSelect on click", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    render(
      <ComposerPalette
        items={DEFAULT_SLASH_COMMANDS}
        kind="slash"
        onClose={onClose}
        onSelect={onSelect}
        query=""
        selectedIndex={0}
      />,
    );

    expect(screen.getByRole("listbox", { name: "Slash commands" })).toBeInTheDocument();
    expect(screen.getByText("/model")).toBeInTheDocument();
    expect(screen.getByText("/clear")).toBeInTheDocument();

    fireEvent.click(screen.getByText("/model"));
    expect(onSelect).toHaveBeenCalledWith(
      expect.objectContaining({
        title: "/model",
      }),
    );
  });

  it("shows empty state when no items match query", () => {
    render(
      <ComposerPalette
        items={[]}
        kind="mention"
        onClose={() => {}}
        onSelect={() => {}}
        query="nonexistent"
        selectedIndex={0}
      />,
    );

    expect(screen.getByText(/No matching files or resources found/i)).toBeInTheDocument();
  });
});

describe("Composer power features integration", () => {
  it("triggers slash palette when typing / at the start of input", () => {
    render(<Composer onSubmit={() => {}} />);
    const textarea = screen.getByRole("textbox", { name: "Message xFalcon" });

    fireEvent.change(textarea, { target: { value: "/m" } });
    expect(screen.getByRole("listbox", { name: "Slash commands" })).toBeInTheDocument();
    expect(screen.getByText("/model")).toBeInTheDocument();
    expect(screen.getByText("/mcp")).toBeInTheDocument();
  });

  it("triggers mention palette when typing @ in input", () => {
    render(<Composer onSubmit={() => {}} projectFiles={["sales_data.csv", "customers.parquet"]} />);
    const textarea = screen.getByRole("textbox", { name: "Message xFalcon" });

    fireEvent.change(textarea, { target: { value: "Analyze @sal" } });
    expect(screen.getByRole("listbox", { name: "Mention suggestions" })).toBeInTheDocument();
    expect(screen.getByText("@sales_data.csv")).toBeInTheDocument();
    expect(textarea).toHaveAttribute("aria-expanded", "true");
    expect(textarea).toHaveAttribute("aria-controls", "composer-palette-list");
  });

  it("does not trigger mention palette when typing email addresses", () => {
    render(<Composer onSubmit={() => {}} projectFiles={["sales_data.csv"]} />);
    const textarea = screen.getByRole("textbox", { name: "Message xFalcon" });

    fireEvent.change(textarea, { target: { value: "Contact user@example.com" } });
    expect(screen.queryByRole("listbox", { name: "Mention suggestions" })).not.toBeInTheDocument();
    expect(textarea).toHaveAttribute("aria-expanded", "false");
  });
});
