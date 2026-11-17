import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EmptyConversation } from "./EmptyConversation";

afterEach(cleanup);

describe("EmptyConversation", () => {
  it("fires a starter prompt when suggestions are enabled", async () => {
    const onSuggest = vi.fn();
    render(<EmptyConversation canSuggest onSuggest={onSuggest} />);
    await screen.getByRole("button", { name: /Summarize what I can do/i }).click();
    expect(onSuggest).toHaveBeenCalledWith("Summarize what I can do in this workspace");
  });

  it("shows the same four starters as xfalcon-web", () => {
    render(<EmptyConversation canSuggest onSuggest={vi.fn()} />);
    for (const prompt of [
      "Summarize what I can do in this workspace",
      "Walk me through the connected data sources",
      "Show what tools I have access to right now",
      "Suggest three questions worth asking my data",
    ]) {
      expect(screen.getByRole("button", { name: prompt })).toBeInTheDocument();
    }
  });

  it("greets the signed-in user by name", () => {
    render(<EmptyConversation canSuggest viewerName="jeevan.tm" onSuggest={vi.fn()} />);
    expect(screen.getByRole("heading", { name: "What can I help you with, jeevan.tm?" })).toBeInTheDocument();
  });

  it("falls back to a generic greeting without an identity", () => {
    render(<EmptyConversation canSuggest onSuggest={vi.fn()} />);
    expect(screen.getByRole("heading", { name: "What can I help you with?" })).toBeInTheDocument();
  });

  it("renders the composer between the greeting and the starters", () => {
    render(<EmptyConversation canSuggest composer={<textarea aria-label="Message xFalcon" />} onSuggest={vi.fn()} />);
    expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toBeInTheDocument();
  });

  it("routes the provider-key hint to settings", async () => {
    const onOpenSettings = vi.fn();
    render(<EmptyConversation canSuggest={false} onOpenSettings={onOpenSettings} onSuggest={vi.fn()} />);
    await screen.getByRole("button", { name: /provider API key in Settings/i }).click();
    expect(onOpenSettings).toHaveBeenCalled();
  });
});
