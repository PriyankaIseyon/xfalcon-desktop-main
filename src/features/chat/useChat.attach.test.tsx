import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ChatConversationView,
  ChatSubmitResult,
  ChatWorkspaceBootstrap,
  ConversationId,
  UploadQueueV1,
} from "../../lib/generated/contracts";
import {
  chatConversationCreate,
  chatConversationDelete,
  chatConversationOpen,
  chatConversationPreview,
  chatMessageSend,
  chatWorkspaceBootstrap,
  uploadChoose,
  uploadPaste,
} from "../../lib/native";
import { listen } from "@tauri-apps/api/event";
import { getChatSnapshot, resetChatStore } from "./chatStore";
import { useChat } from "./useChat";
import { Composer } from "./Composer";
import { MAX_CHAT_ATTACHMENT_BYTES } from "./clipboardAttachments";
import { getUploadQueue, resetUploadQueueForTests } from "../uploads/uploadQueueStore";

// The send path stubs the Tauri global so the workspace refresh runs; the
// chat event bridge starts with it and needs a listener to attach to.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

vi.mock("../../lib/native", () => ({
  chatConversationCreate: vi.fn(),
  chatConversationDelete: vi.fn(),
  chatConversationOpen: vi.fn(),
  chatConversationPreview: vi.fn(),
  chatMcpSelectionGet: vi.fn().mockResolvedValue({ version: 1, server_id: null }),
  chatConversationPin: vi.fn(),
  chatMessageEdit: vi.fn(),
  chatMessageRegenerate: vi.fn(),
  chatMessageSend: vi.fn(),
  chatRunCancel: vi.fn(),
  chatRunRetry: vi.fn(),
  chatWorkspaceBootstrap: vi.fn(),
  uploadCancel: vi.fn(),
  uploadChoose: vi.fn(),
  uploadPaste: vi.fn(),
  uploadRetry: vi.fn(),
}));

beforeEach(() => vi.clearAllMocks());

afterEach(() => {
  cleanup();
  resetChatStore();
  resetUploadQueueForTests();
  vi.restoreAllMocks();
});

function createdView(): ChatConversationView {
  return {
    version: 1,
    conversation_id: "conv-new" as ConversationId,
    title: "New conversation",
    availability: "live",
    messages: [],
    active_run_id: null,
  };
}

function Harness() {
  const chat = useChat(true);
  return (
    <button type="button" onClick={() => void chat.attachFile()}>
      Attach
    </button>
  );
}

function reservedQueue(conversationId = "conv-reserved", id = "upload-1"): UploadQueueV1 {
  return {
    version: 1,
    revision: 1,
    uploads: [
      {
        version: 1,
        id,
        purpose: "chat_attachment",
        state: "ready",
        bytes_sent: 7,
        size: 7,
        display_name: "report.csv",
        safe_error: null,
        conversation_id: conversationId,
      },
    ],
  };
}

function reservedView(): ChatConversationView {
  return { ...createdView(), conversation_id: "conv-reserved" as ConversationId };
}

describe("useChat attach", () => {
  beforeEach(() => {
    vi.mocked(chatWorkspaceBootstrap).mockResolvedValue({
      version: 1,
      conversations: [],
    } satisfies ChatWorkspaceBootstrap);
  });

  it("reserves a new chat's files with the selected MCP server", async () => {
    vi.mocked(uploadChoose).mockResolvedValue(reservedQueue());
    const { result } = renderHook(() => useChat(true, false));
    await act(async () => result.current.selectMcpServer("server-a"));
    await act(async () => result.current.attachFile());
    expect(uploadChoose).toHaveBeenCalledWith(null, "server-a");
  });

  it("releases a ready attachment reservation when the selection changes", async () => {
    vi.mocked(uploadChoose).mockResolvedValue(reservedQueue());
    const { result } = renderHook(() => useChat(true, false));
    await act(async () => result.current.selectMcpServer("server-a"));
    await act(async () => result.current.attachFile());
    expect(getChatSnapshot().reservedConversationId).toBe("conv-reserved");

    await act(async () => result.current.selectMcpServer("server-b"));

    expect(getChatSnapshot()).toMatchObject({ conversationId: null, reservedConversationId: null, messages: [] });
    await waitFor(() => expect(chatConversationDelete).toHaveBeenCalledWith("conv-reserved"));
    expect(getUploadQueue().uploads).toHaveLength(1);
    expect(getUploadQueue().uploads[0]?.conversation_id).toBe("conv-reserved");
    expect(getUploadQueue().uploads.some((upload) => upload.conversation_id === null)).toBe(false);
  });

  afterEach(() => vi.unstubAllGlobals());

  it("opens the picker first and keeps the person on the new-chat screen", async () => {
    const user = userEvent.setup();
    vi.mocked(uploadChoose).mockResolvedValue(reservedQueue());

    render(<Harness />);
    await user.click(screen.getByRole("button", { name: "Attach" }));

    await waitFor(() => expect(getChatSnapshot().reservedConversationId).toBe("conv-reserved"));
    expect(uploadChoose).toHaveBeenCalledWith(null, null);
    expect(chatConversationCreate).not.toHaveBeenCalled();
    expect(chatConversationOpen).not.toHaveBeenCalled();
    expect(getChatSnapshot().conversationId).toBeNull();
    expect(getUploadQueue().uploads[0]?.conversation_id).toBe("conv-reserved");
  });

  it("leaves nothing behind when the picker is cancelled", async () => {
    const user = userEvent.setup();
    vi.mocked(uploadChoose).mockRejectedValue(new Error("no file was chosen"));

    render(<Harness />);
    await user.click(screen.getByRole("button", { name: "Attach" }));

    await waitFor(() => expect(uploadChoose).toHaveBeenCalledWith(null, null));
    expect(getChatSnapshot().reservedConversationId).toBeNull();
    expect(getChatSnapshot().conversationId).toBeNull();
    expect(getChatSnapshot().error).toBeNull();
    expect(chatConversationCreate).not.toHaveBeenCalled();
  });

  it("attaches into the open conversation when there is one", async () => {
    vi.mocked(uploadChoose).mockResolvedValue(reservedQueue("conv-open"));
    const { result } = renderHook(() => useChat(true, false));
    vi.mocked(chatConversationOpen).mockResolvedValue({
      ...createdView(),
      conversation_id: "conv-open" as ConversationId,
    });
    vi.mocked(chatConversationPreview).mockResolvedValue(null);
    await act(async () => result.current.openConversation("conv-open" as ConversationId));

    await act(async () => result.current.attachFile());

    expect(uploadChoose).toHaveBeenCalledWith("conv-open", null);
    expect(getChatSnapshot().reservedConversationId).toBeNull();
  });

  it("sends into the reserved conversation, and only then opens it", async () => {
    vi.mocked(uploadChoose).mockResolvedValue(reservedQueue());
    vi.mocked(chatConversationOpen).mockResolvedValue(reservedView());
    vi.mocked(chatMessageSend).mockResolvedValue({
      version: 1,
      status: "submitted",
      conversation_id: "conv-reserved" as ConversationId,
      run_id: "run-1",
      message: "",
    } satisfies ChatSubmitResult);
    const { result } = renderHook(() => useChat(true, false));

    await act(async () => result.current.attachFile());
    expect(chatConversationOpen).not.toHaveBeenCalled();
    await act(async () => {
      await result.current.send("What is in this file?", ["attachment-1"]);
    });

    expect(chatConversationOpen).toHaveBeenCalledWith("conv-reserved");
    expect(chatConversationCreate).not.toHaveBeenCalled();
    expect(chatMessageSend).toHaveBeenCalledWith("conv-reserved", "What is in this file?", ["attachment-1"], null);
    expect(getChatSnapshot().conversationId).toBe("conv-reserved");
    expect(getChatSnapshot().reservedConversationId).toBeNull();
    expect(chatConversationDelete).not.toHaveBeenCalled();
  });

  it("refreshes the sidebar once a send into a reserved conversation has settled", async () => {
    vi.stubGlobal("__TAURI_INTERNALS__", {});
    vi.mocked(listen).mockResolvedValue(() => {});
    vi.mocked(uploadChoose).mockResolvedValue(reservedQueue());
    vi.mocked(chatConversationOpen).mockResolvedValue(reservedView());
    let accept!: (result: ChatSubmitResult) => void;
    vi.mocked(chatMessageSend).mockReturnValue(
      new Promise((done) => {
        accept = done;
      }),
    );
    const { result } = renderHook(() => useChat(true, false));
    await act(async () => result.current.attachFile());

    let pending!: Promise<void>;
    act(() => {
      pending = result.current.send("What is in this file?", ["attachment-1"]).then(() => undefined);
    });
    await waitFor(() => expect(chatMessageSend).toHaveBeenCalledTimes(1));
    expect(chatWorkspaceBootstrap).not.toHaveBeenCalled();

    await act(async () => {
      accept({
        version: 1,
        status: "submitted",
        conversation_id: "conv-reserved" as ConversationId,
        run_id: "run-1",
        message: "",
      });
      await pending;
    });

    await waitFor(() => expect(chatWorkspaceBootstrap).toHaveBeenCalledTimes(1));
  });

  it("releases a conversation reserved after the person left while the picker was open", async () => {
    let resolve!: (queue: UploadQueueV1) => void;
    vi.mocked(uploadChoose).mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    vi.mocked(chatConversationDelete).mockResolvedValue(undefined);
    const { result } = renderHook(() => useChat(true, false));
    let pending!: Promise<void>;
    act(() => {
      pending = result.current.attachFile();
    });
    await act(async () => result.current.startNewConversation());
    await act(async () => {
      resolve(reservedQueue());
      await pending;
    });

    expect(getChatSnapshot().reservedConversationId).toBeNull();
    await waitFor(() => expect(chatConversationDelete).toHaveBeenCalledWith("conv-reserved"));
  });

  it.each(["new", "open"])("releases the reservation when the person leaves for a %s chat", async (navigation) => {
    vi.mocked(uploadChoose).mockResolvedValue(reservedQueue());
    vi.mocked(chatConversationDelete).mockResolvedValue(undefined);
    const other = { ...createdView(), conversation_id: "conv-other" as ConversationId };
    vi.mocked(chatConversationOpen).mockResolvedValue(other);
    vi.mocked(chatConversationPreview).mockResolvedValue(null);
    const { result } = renderHook(() => useChat(true, false));

    await act(async () => result.current.attachFile());
    await act(async () => {
      if (navigation === "open") await result.current.openConversation(other.conversation_id);
      else await result.current.startNewConversation();
    });

    await waitFor(() => expect(chatConversationDelete).toHaveBeenCalledWith("conv-reserved"));
    expect(getChatSnapshot().reservedConversationId).toBeNull();
  });
});

function PasteHarness() {
  const chat = useChat(true);
  const [visible, setVisible] = useState(true);
  return (
    <>
      <button onClick={() => setVisible(!visible)}>Toggle composer</button>
      {visible && (
        <Composer onSubmit={chat.send} onPasteFiles={chat.pasteFiles} attachmentPending={chat.pastingFiles} />
      )}
    </>
  );
}

describe("useChat clipboard attachments", () => {
  beforeEach(() => {
    vi.mocked(chatWorkspaceBootstrap).mockResolvedValue({ version: 1, conversations: [] });
    vi.mocked(chatConversationCreate).mockResolvedValue(createdView());
    vi.mocked(uploadPaste).mockResolvedValue({ version: 1, revision: 1, uploads: [] });
  });

  it("reserves a conversation only for pasted bytes, without opening it", async () => {
    vi.mocked(uploadPaste).mockResolvedValue({
      version: 1,
      revision: 1,
      uploads: [
        {
          version: 1,
          id: "upload-1",
          purpose: "chat_attachment",
          state: "selected",
          bytes_sent: 0,
          size: 7,
          display_name: "report.csv",
          safe_error: null,
          conversation_id: "conv-new",
        },
      ],
    });
    render(<PasteHarness />);
    fireEvent.paste(screen.getByRole("textbox", { name: "Message xFalcon" }), {
      clipboardData: { files: [new File(["a,b\n1,2"], "report.csv")], getData: () => "" },
    });
    await waitFor(() => expect(uploadPaste).toHaveBeenCalledWith(null, "report.csv", btoa("a,b\n1,2"), null));
    expect(getUploadQueue().uploads[0]?.id).toBe("upload-1");
    expect(getChatSnapshot().reservedConversationId).toBe("conv-new");
    expect(getChatSnapshot().conversationId).toBeNull();
    expect(chatConversationCreate).not.toHaveBeenCalled();
    expect(uploadChoose).not.toHaveBeenCalled();
  });

  it.each([
    "report.doc",
    "report.DOC",
    "report.docx",
    "report.DOCX",
    "slides.ppt",
    "slides.PPT",
    "slides.pptx",
    "slides.PPTX",
  ])("admits Office file %s through the mounted composer upload path", async (name) => {
    const file = new File(["office bytes"], name, { type: "application/octet-stream" });
    render(<PasteHarness />);
    fireEvent.paste(screen.getByRole("textbox", { name: "Message xFalcon" }), {
      clipboardData: { files: [file], getData: () => "" },
    });

    await waitFor(() => expect(uploadPaste).toHaveBeenCalledWith(null, name, btoa("office bytes"), null));
    expect(chatConversationCreate).not.toHaveBeenCalled();
  });

  it("retains admission across composer remounts and refuses another paste or send", async () => {
    let resolve!: (queue: UploadQueueV1) => void;
    vi.mocked(uploadPaste).mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    render(<PasteHarness />);
    const clipboardData = { files: [new File(["content"], "note.txt")], getData: () => "" };
    fireEvent.paste(screen.getByRole("textbox", { name: "Message xFalcon" }), { clipboardData });
    await waitFor(() => expect(uploadPaste).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "Toggle composer" }));
    fireEvent.click(screen.getByRole("button", { name: "Toggle composer" }));
    const input = screen.getByRole("textbox", { name: "Message xFalcon" });
    fireEvent.change(input, { target: { value: "Review" } });
    fireEvent.paste(input, { clipboardData });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(screen.getByRole("button", { name: "Send message" })).toBeDisabled();
    expect(uploadPaste).toHaveBeenCalledTimes(1);
    expect(chatMessageSend).not.toHaveBeenCalled();
    await act(async () => resolve({ version: 1, revision: 1, uploads: [] }));
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
  });

  it.each(["open", "new"])(
    "abandons a paste that navigation (%s) overtook before its bytes were sent",
    async (navigation) => {
      const other = { ...createdView(), conversation_id: "conv-other" as ConversationId, title: "Other chat" };
      vi.mocked(chatConversationPreview).mockResolvedValue(other);
      vi.mocked(chatConversationOpen).mockResolvedValue(other);
      const { result } = renderHook(() => useChat(true, false));
      let pending!: Promise<void>;
      act(() => {
        pending = result.current.pasteFiles([new File(["content"], "note.txt")]);
      });
      await act(async () => {
        if (navigation === "open") await result.current.openConversation(other.conversation_id);
        else await result.current.startNewConversation();
      });
      await act(async () => pending);

      expect(getChatSnapshot().conversationId).toBe(navigation === "open" ? other.conversation_id : null);
      expect(uploadPaste).not.toHaveBeenCalled();
      expect(getChatSnapshot().reservedConversationId).toBeNull();
    },
  );

  it.each(["empty", "large", "unsupported", "batch"])(
    "rejects %s files before creating a conversation or reading bytes",
    async (kind) => {
      const file = new File(kind === "empty" ? [] : ["content"], kind === "unsupported" ? "payload.exe" : "report.csv");
      if (kind === "large") Object.defineProperty(file, "size", { value: MAX_CHAT_ATTACHMENT_BYTES + 1 });
      render(<PasteHarness />);
      fireEvent.paste(screen.getByRole("textbox", { name: "Message xFalcon" }), {
        clipboardData: { files: kind === "batch" ? [file, file, file, file] : [file], getData: () => "" },
      });
      await waitFor(() => expect(screen.getByRole("textbox", { name: "Message xFalcon" })).toBeEnabled());
      expect(chatConversationCreate).not.toHaveBeenCalled();
      expect(uploadPaste).not.toHaveBeenCalled();
    },
  );
});
