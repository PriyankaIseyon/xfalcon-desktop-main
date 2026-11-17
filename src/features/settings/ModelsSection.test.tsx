import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatModelCatalogView, SessionProjection } from "../../lib/generated/contracts";
import { resetAuthSessionForTests, setAuthSession } from "../auth/authStore";
import { ModelsSection } from "./ModelsSection";

const chatModelCatalogMock = vi.hoisted(() => vi.fn());
const chatDefaultModelSetMock = vi.hoisted(() => vi.fn());
const customModelListMock = vi.hoisted(() => vi.fn());
const invalidateModelCatalogMock = vi.hoisted(() => vi.fn());

vi.mock("../models/useModelCatalog", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../models/useModelCatalog")>()),
  invalidateModelCatalog: invalidateModelCatalogMock,
}));

vi.mock("../models/modelNative", () => ({
  loadModelCatalog: chatModelCatalogMock,
  saveDefaultModel: chatDefaultModelSetMock,
  listCustomModels: customModelListMock,
  createCustomModel: vi.fn(),
  updateCustomModel: vi.fn(),
  deleteCustomModel: vi.fn(),
}));

const catalog: ChatModelCatalogView = {
  version: 1,
  models: [
    {
      version: 1,
      id: "~anthropic/claude-sonnet-latest",
      provider: "openrouter",
      display_name: "Sonnet",
      capabilities: { tools: true, image_input: true, reasoning_summary: true },
    },
    {
      version: 1,
      id: "~x-ai/grok-latest",
      provider: "openrouter",
      display_name: "Grok",
      capabilities: { tools: true, image_input: false, reasoning_summary: true },
    },
    {
      version: 1,
      id: "openai/gpt-5.6-luna",
      provider: "openrouter",
      display_name: "Luna",
      capabilities: { tools: true, image_input: false, reasoning_summary: true },
    },
  ],
  selected_model_id: "~anthropic/claude-sonnet-latest",
  selection_locked: false,
};

function authenticated(userId: string): SessionProjection {
  return {
    status: "authenticated",
    version: 1,
    expires_at_ms: 1_000,
    identity: {
      version: 1,
      user_id: userId,
      tenant_id: "tenant",
      display_name: userId,
      email: null,
    },
  };
}

beforeEach(() => {
  customModelListMock.mockResolvedValue([]);
});

afterEach(() => {
  cleanup();
  resetAuthSessionForTests();
  chatModelCatalogMock.mockReset();
  chatDefaultModelSetMock.mockReset();
  customModelListMock.mockReset();
  invalidateModelCatalogMock.mockReset();
});

describe("ModelsSection", () => {
  it("shows a compact single-choice dropdown and saves the default for new chats", async () => {
    chatModelCatalogMock.mockResolvedValueOnce(catalog);
    chatDefaultModelSetMock.mockResolvedValueOnce({
      ...catalog,
      selected_model_id: "~x-ai/grok-latest",
    });
    const user = userEvent.setup();

    render(<ModelsSection />);

    const trigger = await screen.findByRole("button", { name: "Default model" });
    expect(trigger).toHaveTextContent("Sonnet");
    expect(screen.getByText(/Existing chats keep their model/i)).toBeInTheDocument();

    await user.click(trigger);
    expect(screen.getAllByRole("menuitemradio")).toHaveLength(3);
    expect(screen.getByRole("menuitemradio", { name: /Sonnet/ })).toHaveAttribute("aria-checked", "true");
    await user.click(screen.getByRole("menuitemradio", { name: /Grok/ }));

    await waitFor(() => expect(chatDefaultModelSetMock).toHaveBeenCalledWith("~x-ai/grok-latest"));
    await waitFor(() => expect(invalidateModelCatalogMock).toHaveBeenCalledTimes(1));
    expect(trigger).toHaveTextContent("Grok");
  });

  it("drops an unfinished save and unlocks the selector when the account changes", async () => {
    let resolveOldSave: ((value: ChatModelCatalogView) => void) | undefined;
    chatModelCatalogMock
      .mockResolvedValueOnce(catalog)
      .mockResolvedValueOnce({ ...catalog, selected_model_id: "openai/gpt-5.6-luna" });
    chatDefaultModelSetMock.mockReturnValueOnce(
      new Promise<ChatModelCatalogView>((resolve) => {
        resolveOldSave = resolve;
      }),
    );
    setAuthSession(authenticated("account-one"));
    const user = userEvent.setup();

    render(<ModelsSection />);
    await user.click(await screen.findByRole("button", { name: "Default model" }));
    await user.click(screen.getByRole("menuitemradio", { name: /Grok/ }));
    expect(screen.getByRole("button", { name: "Default model" })).toBeDisabled();

    setAuthSession(authenticated("account-two"));

    await waitFor(() => expect(screen.getByRole("button", { name: "Default model" })).toHaveTextContent("Luna"));
    expect(screen.getByRole("button", { name: "Default model" })).toBeEnabled();
    resolveOldSave?.({ ...catalog, selected_model_id: "~x-ai/grok-latest" });
    await Promise.resolve();
    expect(screen.getByRole("button", { name: "Default model" })).toHaveTextContent("Luna");
  });

  it("offers a private model in the same single-choice default and can switch back to OpenRouter", async () => {
    const customModelId = "custom/018f0f45-6d2d-7b41-a741-a488b456805a";
    const withPrivateModel: ChatModelCatalogView = {
      ...catalog,
      models: [
        ...catalog.models,
        {
          version: 1,
          id: customModelId,
          provider: "custom_openai",
          display_name: "Private Qwen",
          capabilities: { tools: true, image_input: false, reasoning_summary: false },
        },
      ],
    };
    chatModelCatalogMock.mockResolvedValueOnce(withPrivateModel);
    chatDefaultModelSetMock
      .mockResolvedValueOnce({ ...withPrivateModel, selected_model_id: customModelId })
      .mockResolvedValueOnce(withPrivateModel);
    const user = userEvent.setup();

    render(<ModelsSection />);
    const trigger = await screen.findByRole("button", { name: "Default model" });
    expect(trigger).toHaveTextContent("Sonnet");
    await user.click(trigger);
    const privateModel = screen.getByRole("menuitemradio", { name: /Private Qwen/ });
    expect(privateModel).toHaveTextContent("Private");
    await user.click(privateModel);

    await waitFor(() => expect(chatDefaultModelSetMock).toHaveBeenCalledWith(customModelId));
    expect(trigger).toHaveTextContent("Private Qwen");

    await user.click(trigger);
    expect(screen.getAllByRole("menuitemradio", { checked: true })).toHaveLength(1);
    await user.click(screen.getByRole("menuitemradio", { name: /Sonnet/ }));
    await waitFor(() => expect(chatDefaultModelSetMock).toHaveBeenLastCalledWith("~anthropic/claude-sonnet-latest"));
    expect(trigger).toHaveTextContent("Sonnet");
  });

  it("restores the saved default when changing providers fails", async () => {
    chatModelCatalogMock.mockResolvedValueOnce(catalog);
    chatDefaultModelSetMock.mockRejectedValueOnce(new Error("save failed"));
    const user = userEvent.setup();

    render(<ModelsSection />);
    const trigger = await screen.findByRole("button", { name: "Default model" });
    await user.click(trigger);
    await user.click(screen.getByRole("menuitemradio", { name: /Grok/ }));

    await waitFor(() => expect(trigger).toHaveTextContent("Sonnet"));
    await user.click(trigger);
    expect(screen.getAllByRole("menuitemradio", { checked: true })).toHaveLength(1);
    expect(screen.getByRole("menuitemradio", { name: /Grok/ })).toHaveAttribute("aria-checked", "false");
  });

  it("keeps one selected option with a large model catalog", async () => {
    const manyModels: ChatModelCatalogView = {
      ...catalog,
      models: [
        ...catalog.models,
        ...Array.from({ length: 47 }, (_, index) => ({
          version: 1 as const,
          id: `custom/model-${index}`,
          provider: "custom_openai" as const,
          display_name: `Private ${index}`,
          capabilities: { tools: true, image_input: false, reasoning_summary: false },
        })),
      ],
    };
    chatModelCatalogMock.mockResolvedValueOnce(manyModels);
    chatDefaultModelSetMock.mockResolvedValueOnce({ ...manyModels, selected_model_id: "custom/model-46" });
    const user = userEvent.setup();

    render(<ModelsSection />);
    const trigger = await screen.findByRole("button", { name: "Default model" });
    await user.click(trigger);
    expect(screen.getAllByRole("menuitemradio")).toHaveLength(50);
    await user.click(screen.getByRole("menuitemradio", { name: /Private 46/ }));

    await waitFor(() => expect(chatDefaultModelSetMock).toHaveBeenCalledWith("custom/model-46"));
    expect(trigger).toHaveTextContent("Private 46");
    await user.click(trigger);
    expect(screen.getAllByRole("menuitemradio", { checked: true })).toHaveLength(1);
  });

  it("names where each model runs with its provider's badge and screen-reader text", async () => {
    const bySource: ChatModelCatalogView = {
      ...catalog,
      models: [
        ...catalog.models,
        {
          version: 1,
          id: "custom/private-qwen",
          provider: "custom_openai",
          display_name: "Private Qwen",
          capabilities: { tools: true, image_input: false, reasoning_summary: false },
        },
        {
          version: 1,
          id: "claude_code/claude-opus-5-5",
          provider: "claude_code",
          display_name: "Opus",
          capabilities: { tools: true, image_input: true, reasoning_summary: true },
        },
        {
          version: 1,
          id: "chatgpt_plan/gpt-6-luna",
          provider: "chatgpt_plan",
          display_name: "GPT Luna",
          capabilities: { tools: true, image_input: false, reasoning_summary: true },
        },
      ],
    };
    chatModelCatalogMock.mockResolvedValueOnce(bySource);
    const user = userEvent.setup();

    render(<ModelsSection />);
    await user.click(await screen.findByRole("button", { name: "Default model" }));

    const expected: Array<[RegExp, string, string]> = [
      [/^Sonnet,/, "OpenRouter", "OpenRouter model"],
      [/Private Qwen/, "Private", "private model"],
      [/Opus/, "Claude account", "Claude account model"],
      [/GPT Luna/, "ChatGPT plan", "ChatGPT plan model"],
    ];
    for (const [name, badge, spoken] of expected) {
      const option = screen.getByRole("menuitemradio", { name });
      expect(option).toHaveTextContent(badge);
      expect(within(option).getByText(`, ${spoken}`)).toBeInTheDocument();
    }
  });

  it("headings the dropdown by where each model runs, OpenRouter before Claude account", async () => {
    const claudeSonnet = {
      version: 1,
      id: "claude_code/claude-sonnet-5",
      provider: "claude_code",
      display_name: "Sonnet",
      capabilities: { tools: true, image_input: true, reasoning_summary: true },
    };
    // The Claude model comes first in the catalog, so the heading order is the sort's doing.
    chatModelCatalogMock.mockResolvedValueOnce({ ...catalog, models: [claudeSonnet, ...catalog.models] });
    const user = userEvent.setup();

    render(<ModelsSection />);
    await user.click(await screen.findByRole("button", { name: "Default model" }));

    // Headings are the menu's rows without a role; options carry menuitemradio and the separator carries separator.
    const headings = Array.from(screen.getByRole("menu").children)
      .filter((row) => row.getAttribute("role") === null)
      .map((row) => row.textContent);
    expect(headings).toEqual(["OpenRouter", "Claude account"]);
    expect(screen.getAllByRole("menuitemradio", { name: /^Sonnet,/ })).toHaveLength(2);
  });
});
