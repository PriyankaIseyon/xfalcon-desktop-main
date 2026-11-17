import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetComposerDraft } from "./composerStore";
import { applyUploadQueue, resetUploadQueueForTests } from "../uploads/uploadQueueStore";
import { ComposerDock } from "./ComposerDock";

const chatModelCatalogMock = vi.hoisted(() => vi.fn());

vi.mock("../models/modelNative", () => ({
  loadModelCatalog: chatModelCatalogMock,
  saveDefaultModel: vi.fn(),
}));

const mcpServerListMock = vi.hoisted(() => vi.fn());
vi.mock("../../lib/native", () => ({
  mcpServerList: mcpServerListMock,
}));

afterEach(() => {
  cleanup();
  resetComposerDraft();
  resetUploadQueueForTests();
  chatModelCatalogMock.mockReset();
  mcpServerListMock.mockReset().mockResolvedValue({
    version: 1,
    servers: [
      {
        version: 1,
        id: "server-a",
        name: "warehouse",
        display_name: "Warehouse",
        enabled: true,
        permission_mode: "allow",
        unavailable: null,
      },
    ],
  });
});

beforeEach(() => {
  chatModelCatalogMock.mockReset().mockResolvedValue({ version: 1, models: [], selected_model_id: null });
  mcpServerListMock.mockReset().mockResolvedValue({
    version: 1,
    servers: [
      {
        version: 1,
        id: "server-a",
        name: "warehouse",
        display_name: "Warehouse",
        enabled: true,
        permission_mode: "allow",
        unavailable: null,
      },
    ],
  });
});

describe("ComposerDock model status", () => {
  it("loads and shows the immutable model for the open conversation", async () => {
    chatModelCatalogMock.mockResolvedValueOnce({
      version: 1,
      models: [
        {
          version: 1,
          id: "~x-ai/grok-latest",
          provider: "openrouter",
          display_name: "Grok",
          capabilities: { tools: true, image_input: false, reasoning_summary: true },
        },
      ],
      selected_model_id: "~x-ai/grok-latest",
      selection_locked: true,
    });

    render(<ComposerDock conversationId="conversation-1" onSubmit={vi.fn()} />);

    expect(await screen.findByLabelText("Model: Grok")).toHaveTextContent("Grok");
    expect(chatModelCatalogMock).toHaveBeenCalledWith("conversation-1");
    expect(screen.queryByRole("button", { name: /grok/i })).not.toBeInTheDocument();
  });

  it("notes the plan a selected ChatGPT plan model runs on, and only that model", async () => {
    chatModelCatalogMock.mockResolvedValueOnce({
      version: 1,
      models: [
        {
          version: 1,
          id: "chatgpt_plan/gpt-6-luna",
          provider: "chatgpt_plan",
          display_name: "GPT Luna",
          capabilities: { tools: true, image_input: false, reasoning_summary: true },
        },
      ],
      selected_model_id: "chatgpt_plan/gpt-6-luna",
      selection_locked: false,
    });

    render(<ComposerDock conversationId="conversation-plan" onSubmit={vi.fn()} />);

    expect(await screen.findByLabelText("Model: GPT Luna")).toBeInTheDocument();
    expect(screen.getByText("Using ChatGPT plan")).toBeInTheDocument();
  });

  it("notes the Claude account a selected Claude model runs on, and not the ChatGPT plan", async () => {
    chatModelCatalogMock.mockResolvedValueOnce({
      version: 1,
      models: [
        {
          version: 1,
          id: "claude_code/claude-sonnet-5",
          provider: "claude_code",
          display_name: "Sonnet",
          capabilities: { tools: true, image_input: true, reasoning_summary: true },
        },
      ],
      selected_model_id: "claude_code/claude-sonnet-5",
      selection_locked: false,
    });

    render(<ComposerDock conversationId="conversation-claude" onSubmit={vi.fn()} />);

    expect(await screen.findByLabelText("Model: Sonnet")).toBeInTheDocument();
    expect(screen.getByText("Using Claude account")).toBeInTheDocument();
    expect(screen.queryByText("Using ChatGPT plan")).not.toBeInTheDocument();
  });

  it("shows no plan note when the selected model is not a plan model", async () => {
    chatModelCatalogMock.mockResolvedValueOnce({
      version: 1,
      models: [
        {
          version: 1,
          id: "~x-ai/grok-latest",
          provider: "openrouter",
          display_name: "Grok",
          capabilities: { tools: true, image_input: false, reasoning_summary: true },
        },
      ],
      selected_model_id: "~x-ai/grok-latest",
      selection_locked: false,
    });

    render(<ComposerDock conversationId="conversation-key" onSubmit={vi.fn()} />);

    expect(await screen.findByLabelText("Model: Grok")).toBeInTheDocument();
    expect(screen.queryByText("Using ChatGPT plan")).not.toBeInTheDocument();
  });
});

describe("ComposerDock MCP server selection", () => {
  it("offers All servers and a configured server through the accessible menu", async () => {
    const onSelect = vi.fn();
    render(<ComposerDock onSubmit={vi.fn()} onMcpServerSelect={onSelect} />);

    const trigger = await screen.findByRole("button", { name: "MCP server" });
    expect(trigger).toHaveTextContent("All servers");
    fireEvent.click(trigger);
    expect(screen.getByRole("menuitemradio", { name: "All servers" })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("menuitemradio", { name: "Warehouse" }));
    expect(onSelect).toHaveBeenCalledWith("server-a");
  });

  it("keeps a removed selected server visible as unavailable", async () => {
    mcpServerListMock.mockResolvedValueOnce({ version: 1, servers: [] });
    render(<ComposerDock mcpServerId="removed-server" onSubmit={vi.fn()} onMcpServerSelect={vi.fn()} />);

    const trigger = await screen.findByRole("button", { name: "MCP server" });
    expect(trigger).toHaveTextContent("Unavailable · removed-server");
    fireEvent.click(trigger);
    expect(screen.getByRole("menuitemradio", { name: /unavailable/i })).toHaveAttribute("aria-disabled", "true");
  });

  it("does not offer a denied server and keeps an already selected one unavailable", async () => {
    mcpServerListMock.mockResolvedValueOnce({
      version: 1,
      servers: [
        {
          version: 1,
          id: "server-denied",
          name: "blocked",
          display_name: "Blocked server",
          enabled: true,
          permission_mode: "deny",
          unavailable: null,
        },
      ],
    });
    render(<ComposerDock mcpServerId="server-denied" onSubmit={vi.fn()} onMcpServerSelect={vi.fn()} />);

    const trigger = await screen.findByRole("button", { name: "MCP server" });
    expect(trigger).toHaveTextContent("Unavailable · server-denied");
    fireEvent.click(trigger);
    expect(screen.queryByRole("menuitemradio", { name: "Blocked server" })).not.toBeInTheDocument();
    expect(screen.getByRole("menuitemradio", { name: /unavailable/i })).toHaveAttribute("aria-disabled", "true");
  });

  it("shows an unavailable state instead of falling back to All servers after a lookup error", async () => {
    render(
      <ComposerDock mcpSelectionError="Could not read selection" onSubmit={vi.fn()} onMcpServerSelect={vi.fn()} />,
    );
    const trigger = await screen.findByRole("button", { name: "MCP server" });
    expect(trigger).toHaveTextContent("Selection unavailable");
    expect(trigger).toBeDisabled();
    expect(trigger).not.toHaveTextContent("All servers");
  });
});

describe("ComposerDock drafts", () => {
  it("does not carry what was typed in one chat into another", () => {
    chatModelCatalogMock.mockResolvedValue({
      version: 1,
      models: [],
      selected_model_id: null,
      selection_locked: false,
    });
    const { rerender } = render(<ComposerDock conversationId="chat-a" onSubmit={vi.fn()} />);
    const input = () => screen.getByRole("textbox") as HTMLTextAreaElement;

    fireEvent.change(input(), { target: { value: "half a question" } });
    expect(input().value).toBe("half a question");

    rerender(<ComposerDock conversationId="chat-b" onSubmit={vi.fn()} />);
    expect(input().value).toBe("");

    rerender(<ComposerDock conversationId="chat-a" onSubmit={vi.fn()} />);
    expect(input().value).toBe("half a question");
  });

  it("keeps the new-chat draft when a reserved conversation holds its files", () => {
    chatModelCatalogMock.mockResolvedValue({
      version: 1,
      models: [],
      selected_model_id: null,
      selection_locked: false,
    });
    const { rerender } = render(<ComposerDock conversationId={null} draftOwner={null} onSubmit={vi.fn()} />);
    const input = () => screen.getByRole("textbox") as HTMLTextAreaElement;

    fireEvent.change(input(), { target: { value: "about my file" } });
    rerender(<ComposerDock conversationId="reserved-1" draftOwner={null} onSubmit={vi.fn()} />);

    expect(input().value).toBe("about my file");
  });
});

describe("ComposerDock stays live while the transcript streams", () => {
  it("shows an attachment as its upload changes and only for its own conversation", () => {
    chatModelCatalogMock.mockResolvedValue({
      version: 1,
      models: [],
      selected_model_id: null,
      selection_locked: false,
    });
    const upload = (id: string, state: "ready" | "cancelled", conversationId: string) => ({
      version: 1 as const,
      id,
      purpose: "chat_attachment" as const,
      state,
      bytes_sent: 10,
      size: 10,
      display_name: `${id}.pdf`,
      safe_error: null,
      conversation_id: conversationId,
    });
    render(<ComposerDock conversationId="chat-a" onSubmit={vi.fn()} />);

    act(() =>
      applyUploadQueue({
        version: 1,
        revision: 1,
        uploads: [upload("notes", "ready", "chat-a"), upload("other", "ready", "chat-b")],
      }),
    );
    expect(screen.getByText("notes.pdf")).toBeInTheDocument();
    expect(screen.queryByText("other.pdf")).not.toBeInTheDocument();

    act(() => applyUploadQueue({ version: 1, revision: 2, uploads: [upload("notes", "cancelled", "chat-a")] }));
    expect(screen.queryByText("notes.pdf")).not.toBeInTheDocument();
  });

  it("shows the model of the conversation it is switched to", async () => {
    const model = (id: string, name: string) => ({
      version: 1,
      models: [
        {
          version: 1,
          id,
          provider: "openrouter",
          display_name: name,
          capabilities: { tools: true, image_input: false, reasoning_summary: true },
        },
      ],
      selected_model_id: id,
      selection_locked: true,
    });
    chatModelCatalogMock.mockResolvedValueOnce(model("model-a", "Alpha"));
    chatModelCatalogMock.mockResolvedValueOnce(model("model-b", "Beta"));
    const { rerender } = render(<ComposerDock conversationId="chat-a" onSubmit={vi.fn()} />);
    expect(await screen.findByLabelText("Model: Alpha")).toBeInTheDocument();

    rerender(<ComposerDock conversationId="chat-b" onSubmit={vi.fn()} />);
    expect(await screen.findByLabelText("Model: Beta")).toBeInTheDocument();
  });

  it("follows the disabled and streaming props", () => {
    chatModelCatalogMock.mockResolvedValue({
      version: 1,
      models: [],
      selected_model_id: null,
      selection_locked: false,
    });
    const onStop = vi.fn();
    const { rerender } = render(<ComposerDock conversationId="chat-a" onSubmit={vi.fn()} onStop={onStop} />);
    const input = () => screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(input()).toBeEnabled();

    rerender(<ComposerDock conversationId="chat-a" onSubmit={vi.fn()} onStop={onStop} disabled />);
    expect(input()).toBeDisabled();

    rerender(<ComposerDock conversationId="chat-a" onSubmit={vi.fn()} onStop={onStop} streaming />);
    expect(input()).toBeEnabled();
    expect(screen.getByRole("button", { name: "Stop the assistant's response" })).toBeInTheDocument();
  });
});
