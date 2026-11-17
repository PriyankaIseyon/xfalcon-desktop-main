import { beforeEach, describe, expect, it } from "vitest";

import { getComposerDraft, resetComposerDraft, setComposerDraft, subscribeComposerDraft } from "./composerStore";

describe("composer drafts", () => {
  beforeEach(() => resetComposerDraft());

  it("keeps what was typed in one chat out of every other chat", () => {
    setComposerDraft("half a question", "chat-a");

    expect(getComposerDraft("chat-a")).toBe("half a question");
    expect(getComposerDraft("chat-b")).toBe("");
    expect(getComposerDraft(null)).toBe("");
  });

  it("gives the new-chat screen its own draft", () => {
    setComposerDraft("for a new chat");
    setComposerDraft("for chat a", "chat-a");

    expect(getComposerDraft(null)).toBe("for a new chat");
    expect(getComposerDraft(undefined)).toBe("for a new chat");
    expect(getComposerDraft("chat-a")).toBe("for chat a");
  });

  it("gives a draft back when its chat is opened again", () => {
    setComposerDraft("unsent", "chat-a");
    setComposerDraft("other", "chat-b");

    expect(getComposerDraft("chat-a")).toBe("unsent");
  });

  it("clears one chat's draft without touching another's", () => {
    setComposerDraft("a", "chat-a");
    setComposerDraft("b", "chat-b");
    setComposerDraft("", "chat-a");

    expect(getComposerDraft("chat-a")).toBe("");
    expect(getComposerDraft("chat-b")).toBe("b");
  });

  it("forgets every draft on reset, and says so once", () => {
    setComposerDraft("a", "chat-a");
    setComposerDraft("b");
    let notifications = 0;
    const unsubscribe = subscribeComposerDraft(() => {
      notifications += 1;
    });

    resetComposerDraft();
    resetComposerDraft();

    expect(getComposerDraft("chat-a")).toBe("");
    expect(getComposerDraft()).toBe("");
    expect(notifications).toBe(1);
    unsubscribe();
  });
});
