import { afterEach, describe, expect, it, vi } from "vitest";
import { resetAuthSessionForTests, setAuthSession } from "../auth/authStore";
import {
  applyChatProjectionForTests,
  applyConversationView,
  getChatSnapshot,
  resetChatStore,
  setWorkspaceBootstrap,
  startChatEventBridge,
  stopChatEventBridge,
} from "./chatStore";

const { listenMock } = vi.hoisted(() => ({ listenMock: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));

function signIn(user: string) {
  setAuthSession({
    version: 1,
    status: "authenticated",
    identity: { version: 1, tenant_id: "tenant", user_id: user, display_name: user, email: null },
    expires_at_ms: Date.now() + 3600000,
  });
}
function openChat() {
  setWorkspaceBootstrap([{ version: 1, conversation_id: "chat", title: "Chat", updated_at: "2026-09-08" }]);
  applyConversationView({
    version: 1,
    conversation_id: "chat",
    title: "Chat",
    availability: "live",
    messages: [],
    active_run_id: "run",
  });
}
function removed(account: string) {
  applyChatProjectionForTests({
    type: "conversation_removed",
    version: 1,
    account_id: account,
    conversation_id: "chat",
  });
}

afterEach(async () => {
  await stopChatEventBridge();
  resetAuthSessionForTests();
  resetChatStore();
  vi.unstubAllGlobals();
  listenMock.mockReset();
});

describe("authoritative conversation deletion events", () => {
  it("clears the owning account chat and rejects delayed events after an account switch", () => {
    signIn("first");
    openChat();
    removed("tenant:other");
    expect(getChatSnapshot().conversationId).toBe("chat");
    removed("tenant:first");
    const cleared = getChatSnapshot();
    expect(cleared.conversationId).toBeNull();
    expect(cleared.activeRunId).toBeNull();
    expect(cleared.conversations).toEqual([]);
    removed("tenant:first");
    expect(getChatSnapshot()).toEqual(cleared);
    signIn("second");
    openChat();
    removed("tenant:first");
    expect(getChatSnapshot().conversationId).toBe("chat");
  });

  it("holds concurrent bridge callers until the deletion listener is registered", async () => {
    vi.stubGlobal("__TAURI_INTERNALS__", {});
    let registered!: (unlisten: () => void) => void;
    listenMock.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          registered = resolve;
        }),
    );
    listenMock.mockResolvedValue(() => {});
    const first = startChatEventBridge();
    const second = startChatEventBridge();
    let ready = false;
    void second.then(() => {
      ready = true;
    });
    await Promise.resolve();
    expect(ready).toBe(false);
    expect(second).toBe(first);
    registered(() => {});
    await Promise.all([first, second]);
    expect(ready).toBe(true);
    signIn("first");
    openChat();
    const listener = listenMock.mock.calls.find(([channel]) => channel === "xfalcon://chat-projection-event")?.[1];
    listener({
      payload: { type: "conversation_removed", version: 1, account_id: "tenant:first", conversation_id: "chat" },
    });
    expect(getChatSnapshot().conversationId).toBeNull();
  });
});
