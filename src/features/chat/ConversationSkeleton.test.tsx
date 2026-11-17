import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { ConversationSkeleton } from "./ConversationSkeleton";
import { ChatPane } from "./ChatPane";
import { beginConversationOpen, resetChatStore, setWorkspaceBootstrap } from "./chatStore";

afterEach(() => {
  cleanup();
  resetChatStore();
});

describe("ConversationSkeleton", () => {
  it("announces a cache miss and uses shared reduced-motion-aware skeletons", () => {
    const { container } = render(<ConversationSkeleton />);
    const status = screen.getByRole("status", { name: "Loading conversation" });
    expect(status).toHaveAttribute("aria-busy", "true");
    expect(container.querySelectorAll(".skeleton-shimmer").length).toBeGreaterThan(2);
  });

  it("replaces only the transcript column while an opened conversation misses cache", () => {
    resetChatStore();
    setWorkspaceBootstrap([
      { version: 1, conversation_id: "conv-2", title: "Selected title", updated_at: "2026-09-01T00:00:00Z" },
    ]);
    beginConversationOpen("conv-2");
    render(
      <ChatPane
        activeProjectName={null}
        disabled={false}
        onClearProject={() => {}}
        onDismissNotice={() => {}}
        onOpenSettings={() => {}}
        onResetConversation={() => {}}
        onRetryRun={() => {}}
        onStop={() => {}}
        stopping={false}
        onSubmit={() => {}}
        projectUnavailable={false}
        viewerName="Tester"
      />,
    );

    expect(screen.getByRole("status", { name: "Loading conversation" })).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: "Message xFalcon" })).not.toBeInTheDocument();
  });
});
