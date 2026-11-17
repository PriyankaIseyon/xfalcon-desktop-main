import { beforeEach, describe, expect, it, vi } from "vitest";
import { chatDefaultModelSet, chatModelCatalog } from "./native";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: vi.fn(),
  invoke: invokeMock,
}));

beforeEach(() => invokeMock.mockReset());

describe("model-selection native boundary", () => {
  it("loads a conversation model without accepting a replacement model", async () => {
    invokeMock.mockResolvedValueOnce({});

    await chatModelCatalog("conversation-1");

    expect(invokeMock).toHaveBeenCalledWith("chat_model_catalog", {
      conversationId: "conversation-1",
    });
  });

  it("changes only the account default through a separate command", async () => {
    invokeMock.mockResolvedValueOnce({});

    await chatDefaultModelSet("openai/gpt-5.6-luna");

    expect(invokeMock).toHaveBeenCalledWith("chat_default_model_set", {
      modelId: "openai/gpt-5.6-luna",
    });
  });
});
