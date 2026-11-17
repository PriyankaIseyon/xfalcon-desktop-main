import { useCallback, useEffect, useRef, useState } from "react";
import type { ConversationId, McpServerId, UploadQueueV1 } from "../../lib/generated/contracts";
import { toast } from "../../ui/toastStore";
import { getAuthAccountGeneration } from "../auth/authStore";
import { chatErrorCode, describeChatError } from "./chatPresentation";
import {
  chatConversationCreate,
  chatConversationDelete,
  chatConversationOpen,
  chatConversationPreview,
  chatMcpSelectionGet,
  chatConversationPin,
  chatMessageEdit,
  chatMessageRegenerate,
  chatMessageSend,
  chatRunCancel,
  chatRunRetry,
  chatWorkspaceBootstrap,
  uploadCancel,
  uploadChoose,
  uploadPaste,
  uploadRetry,
} from "../../lib/native";
import {
  addOptimisticUserMessage,
  adoptActiveRun,
  applyConversationPreview,
  applyCreatedConversationView,
  beginConversationOpen,
  beginNewConversation,
  beginRunSubmission,
  beginTurnRevision,
  clearRecoveryNotice,
  dropOptimisticUserMessage,
  finishRunSubmission,
  finishTurnRevision,
  getChatSnapshot,
  completeConversationOpen,
  rollbackConversationOpen,
  rollbackTurnRevision,
  removeConversationFromStore,
  setChatError,
  setReservedConversation,
  setWorkspaceBootstrap,
  startChatEventBridge,
  useChatSelector,
} from "./chatStore";
import { noteMessageSent } from "../delight/milestones";
import { applyUploadQueue, getUploadQueue, startUploadQueueBridge } from "../uploads/uploadQueueStore";
import { getComposerDraft, setComposerDraft } from "./composerStore";
import { clearTurnEdit, getTurnEditState } from "./turnEditStore";
import { clipboardFileBase64, validateClipboardFiles } from "./clipboardAttachments";
import { isBoundRunPending } from "./runState";

const ACTIVE_UPLOAD_STATES = new Set(["selected", "preparing", "uploading", "resigning", "finalizing"]);

/**
 * The chat surface the SHELL needs: the actions, and the state that changes once
 * per conversation or once per run.
 *
 * Deliberately not the whole snapshot. Reading it all is what made a streamed
 * token re-render the entire application; per-token state lives in
 * `useTranscript`, and only `ChatPane` reads it
 * (spec: chat-streaming-performance).
 */
export function useChat(chatReady: boolean | null, bootstrapEnabled = chatReady === true) {
  const conversationId = useChatSelector((snapshot) => snapshot.conversationId);
  const conversations = useChatSelector((snapshot) => snapshot.conversations);
  const title = useChatSelector((snapshot) => snapshot.title);
  const workspaceLoaded = useChatSelector((snapshot) => snapshot.workspaceLoaded);
  const messages = useChatSelector((snapshot) => snapshot.messages);
  const activeRunId = useChatSelector((snapshot) => snapshot.activeRunId);
  const runState = useChatSelector((snapshot) => snapshot.runState);
  const authGeneration = getAuthAccountGeneration();
  const [mcpSelection, setMcpSelection] = useState<{
    conversationId: string | null;
    serverId: McpServerId | null;
    authGeneration: number;
    status: "ready" | "loading" | "error";
    error: string | null;
  }>({ conversationId: null, serverId: null, authGeneration, status: "ready", error: null });
  const mcpSelectionRef = useRef(mcpSelection);
  mcpSelectionRef.current = mcpSelection;
  const selectionRequest = useRef(0);
  const [selectionReloadRevision, setSelectionReloadRevision] = useState(0);
  const mounted = useRef(false);
  const pasteInFlight = useRef(false);
  const attachmentInFlight = useRef(false);
  const selectionChangeInFlight = useRef(false);
  const [pastingFiles, setPastingFiles] = useState(false);
  const [attachingFile, setAttachingFile] = useState(false);
  const accountGeneration = useRef(getAuthAccountGeneration());
  const initialBootstrapStarted = useRef(false);
  const pendingBootstrapError = useRef<string | null>(null);
  const chatReadyRef = useRef(chatReady);
  const openGeneration = useRef(0);
  const activeOpenRestore = useRef<ReturnType<typeof beginConversationOpen>>(null);
  chatReadyRef.current = chatReady;
  // `workspace_status` is an auxiliary readiness summary. A successful chat
  // bootstrap is stronger evidence that the chat data plane is connected. If
  // the two parallel startup calls race and only the summary fails, keep chat
  // operational instead of stranding a proven connection behind `null`.
  const chatAvailable = chatReady === true || (chatReady === null && workspaceLoaded);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const accountRequestIsCurrent = useCallback(
    () => mounted.current && accountGeneration.current === getAuthAccountGeneration(),
    [],
  );

  useEffect(() => {
    if (!chatAvailable) return;
    const request = ++selectionRequest.current;
    if (conversationId === null) {
      if (
        mcpSelectionRef.current.conversationId !== null ||
        mcpSelectionRef.current.authGeneration !== authGeneration
      ) {
        const nextSelection = {
          conversationId: null,
          serverId: null,
          authGeneration,
          status: "ready" as const,
          error: null,
        };
        mcpSelectionRef.current = nextSelection;
        setMcpSelection(nextSelection);
      }
      return;
    }
    setMcpSelection({ conversationId, serverId: null, authGeneration, status: "loading", error: null });
    void chatMcpSelectionGet(conversationId)
      .then((selection) => {
        if (
          request !== selectionRequest.current ||
          authGeneration !== getAuthAccountGeneration() ||
          getChatSnapshot().conversationId !== conversationId
        )
          return;
        setMcpSelection({
          conversationId,
          serverId: selection.server_id,
          authGeneration,
          status: "ready",
          error: null,
        });
      })
      .catch((cause) => {
        if (
          request !== selectionRequest.current ||
          authGeneration !== getAuthAccountGeneration() ||
          getChatSnapshot().conversationId !== conversationId
        )
          return;
        const message = describeChatError(cause, "Could not load this chat's MCP server selection.");
        setMcpSelection({ conversationId, serverId: null, authGeneration, status: "error", error: message });
        setChatError(message);
      });
    return () => {
      if (selectionRequest.current === request) selectionRequest.current += 1;
    };
  }, [authGeneration, chatAvailable, conversationId, selectionReloadRevision]);

  const requireMcpSelection = useCallback((targetId: string | null): McpServerId | null => {
    const selection = mcpSelectionRef.current;
    if (
      selection.conversationId !== targetId ||
      selection.authGeneration !== getAuthAccountGeneration() ||
      selection.status !== "ready"
    ) {
      throw new Error(
        selection.error ??
          (selection.status === "error"
            ? "Could not load this chat's MCP server selection."
            : "This chat's MCP server selection is still loading."),
      );
    }
    return selection.serverId;
  }, []);

  useEffect(() => {
    // Failure is already surfaced through the store's error state; the
    // rejection only needs to not become an unhandled-promise warning.
    startChatEventBridge().catch(() => {});
    startUploadQueueBridge().catch(() => {});
  }, []);

  const loadWorkspace = useCallback(
    async (surfaceFailure: boolean, behindSend = false) => {
      if (!(globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__) {
        return;
      }
      try {
        const workspace = await chatWorkspaceBootstrap();
        if (!accountRequestIsCurrent()) return;
        if (!surfaceFailure && chatReadyRef.current === false) return;
        setWorkspaceBootstrap(workspace.conversations, workspace.recovery_notice);
        // A refresh that follows a send never clears an error: the send
        // cleared its own error when it started, so anything present now came
        // from this send or its run, possibly while this request was in flight.
        if (!behindSend) setChatError(null);
      } catch (cause) {
        if (!accountRequestIsCurrent()) return;
        // Same for a refresh that fails behind a send: if the send's error is
        // on screen when this failure settles, that message is the one the
        // reader needs. The error is read here, not when the refresh starts.
        if (behindSend && getChatSnapshot().error !== null) return;
        const message = describeChatError(cause, "Could not load conversations.");
        if (surfaceFailure || chatReadyRef.current === true) {
          setChatError(message);
        } else if (chatReadyRef.current === null) {
          pendingBootstrapError.current = message;
        }
      }
    },
    [accountRequestIsCurrent],
  );

  const refreshWorkspace = useCallback(async () => {
    if (!chatAvailable) return;
    await loadWorkspace(true);
  }, [chatAvailable, loadWorkspace]);

  useEffect(() => {
    if (!bootstrapEnabled || initialBootstrapStarted.current) return;
    initialBootstrapStarted.current = true;
    void loadWorkspace(false);
  }, [bootstrapEnabled, loadWorkspace]);

  useEffect(() => {
    if (chatReady === null) return;
    if (chatReady && pendingBootstrapError.current) {
      setChatError(pendingBootstrapError.current);
    }
    pendingBootstrapError.current = null;
  }, [chatReady]);

  const cancellingRunRef = useRef<string | null>(null);
  /**
   * The run a Stop press is waiting on, for the control's own busy state.
   *
   * Separate from `cancellingRunRef`, which stays: the ref is the re-entry
   * guard and has to be synchronous, because two presses inside one render
   * would both pass a state check and fire two cancels. This is only what the
   * button draws, so it may lag a frame.
   *
   * It exists because Stop had no acknowledgement at all. `chat_run_cancel`
   * can take several round trips when this process holds no local handle for
   * the run — `get_run`, an access reconcile, the cancel, then a reopen — and
   * for all of that the control sat there looking unpressed.
   */
  const [stoppingRunId, setStoppingRunId] = useState<string | null>(null);
  const cancelRun = useCallback(async () => {
    if (!isBoundRunPending(activeRunId, runState) || cancellingRunRef.current === activeRunId) return;
    cancellingRunRef.current = activeRunId;
    setStoppingRunId(activeRunId);
    try {
      await chatRunCancel(activeRunId);
    } catch (cause) {
      if (!accountRequestIsCurrent()) return;
      const current = getChatSnapshot();
      if (current.activeRunId !== activeRunId || !isBoundRunPending(current.activeRunId, current.runState)) return;
      const message = describeChatError(cause, "Could not stop the response. The secure runtime did not respond.");
      setChatError(message);
      toast.error(message);
    } finally {
      if (cancellingRunRef.current === activeRunId) cancellingRunRef.current = null;
      // Cleared whatever the outcome. On success the run settles and the
      // control becomes Send anyway; on failure the reader has just been told
      // it did not stop, and must be able to press again.
      setStoppingRunId((current) => (current === activeRunId ? null : current));
    }
  }, [accountRequestIsCurrent, activeRunId, runState]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      const target = event.target;
      if (
        target instanceof HTMLElement &&
        (target.isContentEditable || target.tagName === "INPUT" || target.tagName === "TEXTAREA")
      ) {
        return;
      }
      if (isBoundRunPending(activeRunId, runState)) {
        event.preventDefault();
        void cancelRun();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [activeRunId, cancelRun, runState]);

  const send = useCallback(
    async (prompt: string, attachmentIds: string[] = []) => {
      if (!chatAvailable || pasteInFlight.current) return;
      let selectedMcpServerId: McpServerId | null;
      try {
        selectedMcpServerId = requireMcpSelection(conversationId);
      } catch (cause) {
        setChatError(cause instanceof Error ? cause.message : String(cause));
        return;
      }
      const trimmed = prompt.trim();
      if (!trimmed && attachmentIds.length === 0) return;
      const body = trimmed || "Attached files.";
      // Where the person typed it: the chat they sent from, or the chat that
      // was created for it once one exists.
      let draftOwner = conversationId;
      // Set when this send minted or opened the conversation. The sidebar
      // row is refreshed after the send settles, never before it.
      let needsSidebarRefresh = false;
      const restorePrompt = () => {
        if (trimmed && getComposerDraft(draftOwner).trim() === "") {
          setComposerDraft(trimmed, draftOwner);
        }
      };

      // Everything the reader is owed for pressing send happens HERE, in the
      // same task as the keystroke: their turn on screen, and the indicator
      // that the app is working on it. Both used to wait on the backend —
      // creating the conversation, then a send that refreshes the session and
      // reads the model catalog before it can name a run — so a send was
      // followed by an empty screen for as long as that took (spec:
      // chat-run-activity).
      const optimisticId = addOptimisticUserMessage(
        body,
        getUploadQueue()
          .uploads.filter((upload) => upload.resource_id != null && attachmentIds.includes(upload.resource_id))
          .map((upload) => ({
            attachment_id: upload.resource_id as string,
            file_name: upload.display_name,
            mime_type: "",
          })),
      );
      beginRunSubmission();
      setChatError(null);

      try {
        // Web parity (`useSendMessage.ensureConversation`): a conversation row
        // is minted on the first send, never when "New chat" is clicked — so an
        // abandoned new-chat screen leaves nothing behind in anyone's sidebar.
        let targetConversationId = conversationId;
        const reserved = targetConversationId === null ? getChatSnapshot().reservedConversationId : null;
        if (reserved !== null) {
          // Files attached on the new-chat screen already have a conversation.
          // Opening it now is the moment it becomes this chat.
          try {
            const view = await chatConversationOpen(reserved);
            if (!accountRequestIsCurrent()) return undefined;
            applyCreatedConversationView(view);
            setReservedConversation(null);
            targetConversationId = view.conversation_id;
            draftOwner = view.conversation_id;
            setMcpSelection({
              conversationId: view.conversation_id,
              serverId: selectedMcpServerId,
              authGeneration,
              status: "ready",
              error: null,
            });
            needsSidebarRefresh = true;
          } catch (cause) {
            if (!accountRequestIsCurrent()) return undefined;
            dropOptimisticUserMessage(optimisticId);
            restorePrompt();
            setChatError(describeChatError(cause, "Could not open the chat holding your files. Try again."));
            return undefined;
          }
        } else if (targetConversationId === null) {
          try {
            const view = await chatConversationCreate(trimmed || undefined, selectedMcpServerId);
            if (!accountRequestIsCurrent()) return undefined;
            // Carries the row above across the hop into the conversation it
            // just created; an ordinary snapshot would blank it.
            applyCreatedConversationView(view);
            targetConversationId = view.conversation_id;
            draftOwner = view.conversation_id;
            setMcpSelection({
              conversationId: view.conversation_id,
              serverId: selectedMcpServerId,
              authGeneration,
              status: "ready",
              error: null,
            });
            needsSidebarRefresh = true;
          } catch (cause) {
            if (!accountRequestIsCurrent()) return undefined;
            // Nothing was sent, so the turn on screen has no future durable
            // row to replace it.
            dropOptimisticUserMessage(optimisticId);
            restorePrompt();
            setChatError(
              describeChatError(cause, "Could not start a new conversation. The secure runtime did not respond."),
            );
            return undefined;
          }
        }

        // `chat_message_send` reports its own failures as a `failed` result; a
        // rejected invoke means the secure runtime itself never answered (IPC
        // failure, or no Tauri shell at all). Swallowing it leaves a sent
        // message with no visible outcome.
        try {
          const result = await chatMessageSend(targetConversationId, body, attachmentIds, selectedMcpServerId);
          if (!accountRequestIsCurrent()) return undefined;
          if (result.status === "failed") {
            dropOptimisticUserMessage(optimisticId);
            restorePrompt();
            setChatError(result.message);
            return result;
          }
          noteMessageSent();
          if (result.conversation_id && result.run_id) {
            adoptActiveRun(result.conversation_id, result.run_id);
          }
          return result;
        } catch {
          if (!accountRequestIsCurrent()) return undefined;
          dropOptimisticUserMessage(optimisticId);
          restorePrompt();
          setChatError("Could not send the message. The secure runtime did not respond.");
          return undefined;
        }
      } finally {
        // The claim covers exactly this round trip. Past it the run either
        // exists — `adoptActiveRun` has already put it in `queued`, so the
        // indicator carries on without a flicker — or it does not, and the
        // reader is looking at the error that says why.
        if (accountRequestIsCurrent()) finishRunSubmission();
        // A new conversation's row lives in the workspace collection, which
        // the sidebar reads. The refresh starts here, after the send has
        // settled: by now the run is registered as driven by this process, so
        // the bootstrap cannot race its registration. The row appears once the
        // send is accepted, instead of the send waiting for the refresh.
        if (needsSidebarRefresh && accountRequestIsCurrent()) {
          void loadWorkspace(true, true);
        }
      }
    },
    [accountRequestIsCurrent, authGeneration, chatAvailable, conversationId, loadWorkspace, requireMcpSelection],
  );

  /**
   * Cancels a reserved conversation's uploads and deletes it: its files were
   * attached on a new-chat screen the person then left, so nothing was sent
   * and nothing should be left in anyone's sidebar. Best effort -- the person
   * has moved on, so a failure is logged, not shown.
   */
  const releaseReservation = useCallback(async (reserved: string) => {
    const pending = getUploadQueue().uploads.filter(
      (upload) =>
        upload.conversation_id === reserved &&
        upload.state !== "ready" &&
        upload.state !== "failed" &&
        upload.state !== "cancelled",
    );
    try {
      for (const upload of pending) applyUploadQueue(await uploadCancel(upload.id));
      await chatConversationDelete(reserved);
    } catch (cause) {
      console.warn("A conversation reserved for attachments could not be released", cause);
    }
  }, []);

  /** Leaves the reservation behind, unless the person is going into it. */
  const leaveReservation = useCallback(
    (unlessTarget: string | null) => {
      const reserved = getChatSnapshot().reservedConversationId;
      if (reserved === null || reserved === unlessTarget) return;
      setReservedConversation(null);
      void releaseReservation(reserved);
    },
    [releaseReservation],
  );

  /**
   * Queues files for the open conversation, or for the new-chat screen.
   *
   * With no conversation open, the backend opens the picker (or reads the
   * clipboard) first and reserves a conversation only for a file actually
   * chosen -- so Attach never moves the person, and a cancelled picker leaves
   * nothing (spec: chat-attachments). The reservation is whichever
   * conversation the returned queue names that the queue did not hold before.
   */
  const queueAttachments = useCallback(
    async (
      enqueue: (target: string | null, mcpServerId: McpServerId | null) => Promise<UploadQueueV1>,
    ): Promise<boolean> => {
      const snapshot = getChatSnapshot();
      const target = snapshot.conversationId ?? snapshot.reservedConversationId;
      const mcpServerId = requireMcpSelection(snapshot.conversationId);
      const navigation = openGeneration.current;
      const known = new Set(getUploadQueue().uploads.map((upload) => upload.conversation_id));
      const queue = await enqueue(target, mcpServerId);
      if (!accountRequestIsCurrent()) return false;
      applyUploadQueue(queue);
      if (target !== null) return true;
      const reserved = queue.uploads
        .map((upload) => upload.conversation_id)
        .find((id): id is string => id !== null && !known.has(id));
      if (reserved === undefined) return true;
      // The person moved on while the picker was open: the files belong to a
      // screen that no longer exists.
      if (navigation !== openGeneration.current || getChatSnapshot().conversationId !== null) {
        void releaseReservation(reserved);
        return false;
      }
      setReservedConversation(reserved);
      return true;
    },
    [accountRequestIsCurrent, releaseReservation, requireMcpSelection],
  );

  const attachFile = useCallback(async () => {
    const current = getChatSnapshot();
    if (
      !chatAvailable ||
      pasteInFlight.current ||
      attachmentInFlight.current ||
      current.awaitingRun ||
      current.revising ||
      isBoundRunPending(current.activeRunId, current.runState)
    )
      return;
    attachmentInFlight.current = true;
    setAttachingFile(true);
    try {
      await queueAttachments((target, mcpServerId) => uploadChoose(target, mcpServerId));
    } catch (cause) {
      if (!accountRequestIsCurrent()) return;
      const message = cause instanceof Error ? cause.message : String(cause);
      if (message.includes("no file was chosen")) return;
      setChatError(message);
      toast.error(message);
    } finally {
      attachmentInFlight.current = false;
      if (mounted.current) setAttachingFile(false);
    }
  }, [accountRequestIsCurrent, chatAvailable, queueAttachments]);

  const pasteFiles = useCallback(
    async (files: File[]) => {
      if (!chatAvailable || !accountRequestIsCurrent() || pasteInFlight.current) return;
      const snapshot = getChatSnapshot();
      if (snapshot.awaitingRun || snapshot.revising || isBoundRunPending(snapshot.activeRunId, snapshot.runState))
        return;
      pasteInFlight.current = true;
      setPastingFiles(true);
      try {
        validateClipboardFiles(files);
        if (!files.length) {
          await queueAttachments((target, mcpServerId) => uploadPaste(target, null, null, mcpServerId));
          return;
        }
        const start = getChatSnapshot().conversationId;
        const navigation = openGeneration.current;
        for (const file of files) {
          const data = await clipboardFileBase64(file);
          // A paste belongs to the screen it was made on: navigation while a
          // file is still being read ends it. The first file may reserve the
          // conversation; the rest follow it.
          if (
            !accountRequestIsCurrent() ||
            navigation !== openGeneration.current ||
            getChatSnapshot().conversationId !== start
          )
            return;
          if (!(await queueAttachments((target, mcpServerId) => uploadPaste(target, file.name, data, mcpServerId))))
            return;
        }
      } catch (cause) {
        if (!accountRequestIsCurrent()) return;
        const message = cause instanceof Error ? cause.message : String(cause);
        setChatError(message);
        toast.error(message);
      } finally {
        pasteInFlight.current = false;
        if (mounted.current) setPastingFiles(false);
      }
    },
    [accountRequestIsCurrent, chatAvailable, queueAttachments],
  );

  const cancelUpload = useCallback(
    async (uploadId: string) => {
      try {
        const queue = await uploadCancel(uploadId);
        if (!accountRequestIsCurrent()) return;
        applyUploadQueue(queue);
      } catch (cause) {
        if (!accountRequestIsCurrent()) return;
        const message = cause instanceof Error ? cause.message : String(cause);
        toast.error(message);
      }
    },
    [accountRequestIsCurrent],
  );

  const retryUpload = useCallback(
    async (uploadId: string) => {
      try {
        const queue = await uploadRetry(uploadId);
        if (!accountRequestIsCurrent()) return;
        applyUploadQueue(queue);
      } catch (cause) {
        if (!accountRequestIsCurrent()) return;
        const message = cause instanceof Error ? cause.message : String(cause);
        toast.error(message);
      }
    },
    [accountRequestIsCurrent],
  );

  const retryLastRun = useCallback(async () => {
    const runId = activeRunId;
    if (!runId || !chatAvailable || getChatSnapshot().awaitingRun) return;
    let mcpServerId: McpServerId | null;
    try {
      mcpServerId = requireMcpSelection(conversationId);
    } catch (cause) {
      setChatError(cause instanceof Error ? cause.message : String(cause));
      return;
    }

    // Keep the recovery card in place while the native call answers. Taking it
    // away optimistically left dead air, and a rejected invoke had no surface
    // left to restore. The button reports this claim through `awaitingRun`.
    beginRunSubmission();
    try {
      const result = await chatRunRetry(runId, mcpServerId);
      if (!accountRequestIsCurrent()) return;
      if (result.status === "failed") {
        setChatError(result.message);
        return;
      }

      // Success is the point at which both reports of the old death retire.
      clearRecoveryNotice();
      setChatError(null);
      // A genuinely failed run minted a fresh run; rebind so its events pass
      // the scoping. An already-settled run returns no run id because there is
      // nothing to execute again.
      if (result.conversation_id && result.run_id) {
        adoptActiveRun(result.conversation_id, result.run_id);
      }
    } catch (cause) {
      if (!accountRequestIsCurrent()) return;
      // Only a rejected *bridge* is the secure runtime's fault. Every other
      // rejection carries the workspace's own reason, and claiming otherwise is
      // what sent readers to debug this machine during a workspace outage.
      setChatError(describeChatError(cause, "Could not retry the response. The secure runtime did not respond."));
    } finally {
      if (accountRequestIsCurrent()) finishRunSubmission();
    }
  }, [accountRequestIsCurrent, chatAvailable, activeRunId, conversationId, requireMcpSelection]);

  // The surface reports a run's death as ONE card fed by two store fields,
  // so dismissing it has to clear both — clearing one alone brought the card
  // straight back carrying the other.
  const dismissNotice = useCallback(() => {
    clearRecoveryNotice();
    setChatError(null);
  }, []);

  // Web parity: "New chat" only resets the view. The conversation row is
  // created by `send` on the first message, so clicking around never litters
  // the sidebar with empty "New Chat" rows.
  const startNewConversation = useCallback(
    async (nextMcpServerId: McpServerId | null = null) => {
      openGeneration.current += 1;
      activeOpenRestore.current = null;
      leaveReservation(null);
      selectionRequest.current += 1;
      setComposerDraft("", null);
      const nextSelection = {
        conversationId: null,
        serverId: nextMcpServerId,
        authGeneration,
        status: "ready" as const,
        error: null,
      };
      mcpSelectionRef.current = nextSelection;
      setMcpSelection(nextSelection);
      beginNewConversation();
      setChatError(null);
    },
    [authGeneration, leaveReservation],
  );

  const selectMcpServer = useCallback(
    async (nextMcpServerId: McpServerId | null, prepare?: () => Promise<boolean>) => {
      const snapshot = getChatSnapshot();
      const currentSelection = mcpSelectionRef.current;
      const navigationEpoch = openGeneration.current;
      const selectionAccountGeneration = currentSelection.authGeneration;
      const hasTransfer = () =>
        getUploadQueue().uploads.some(
          (upload) =>
            upload.purpose === "chat_attachment" &&
            (upload.conversation_id === snapshot.conversationId ||
              upload.conversation_id === snapshot.reservedConversationId) &&
            ACTIVE_UPLOAD_STATES.has(upload.state),
        );
      if (
        currentSelection.conversationId !== snapshot.conversationId ||
        currentSelection.authGeneration !== getAuthAccountGeneration() ||
        currentSelection.status !== "ready" ||
        snapshot.awaitingRun ||
        snapshot.revising ||
        isBoundRunPending(snapshot.activeRunId, snapshot.runState) ||
        pasteInFlight.current ||
        attachmentInFlight.current ||
        selectionChangeInFlight.current ||
        hasTransfer()
      )
        return;
      if (currentSelection.serverId === nextMcpServerId) return;
      selectionChangeInFlight.current = true;
      try {
        if (prepare && !(await prepare())) return;
        const afterPrepare = getChatSnapshot();
        if (
          navigationEpoch !== openGeneration.current ||
          selectionAccountGeneration !== getAuthAccountGeneration() ||
          afterPrepare.conversationId !== snapshot.conversationId ||
          afterPrepare.awaitingRun ||
          afterPrepare.revising ||
          isBoundRunPending(afterPrepare.activeRunId, afterPrepare.runState) ||
          pasteInFlight.current ||
          attachmentInFlight.current ||
          hasTransfer() ||
          mcpSelectionRef.current.conversationId !== snapshot.conversationId ||
          mcpSelectionRef.current.authGeneration !== getAuthAccountGeneration() ||
          mcpSelectionRef.current.status !== "ready" ||
          mcpSelectionRef.current.serverId !== currentSelection.serverId
        )
          return;
        setComposerDraft("", snapshot.conversationId);
        void startNewConversation(nextMcpServerId);
      } finally {
        selectionChangeInFlight.current = false;
      }
    },
    [startNewConversation],
  );

  // Pin/delete feedback goes through the toast stack, not the transcript's
  // inline error slot — these are sidebar actions, and their outcome should
  // not appear inside whatever conversation happens to be open.
  const pinConversation = useCallback(
    async (targetId: ConversationId, pinned: boolean) => {
      if (!chatAvailable) return;
      try {
        const workspace = await chatConversationPin(targetId, pinned);
        if (!accountRequestIsCurrent()) return;
        setWorkspaceBootstrap(workspace.conversations);
        toast.success(pinned ? "Conversation pinned" : "Conversation unpinned");
      } catch (cause) {
        if (!accountRequestIsCurrent()) return;
        toast.error(describeChatError(cause, `Could not ${pinned ? "pin" : "unpin"} that conversation.`));
      }
    },
    [accountRequestIsCurrent, chatAvailable],
  );

  const deleteConversation = useCallback(
    async (targetId: ConversationId) => {
      if (!chatAvailable) return false;
      try {
        await chatConversationDelete(targetId);
        if (!accountRequestIsCurrent()) return false;
        removeConversationFromStore(targetId);
        await refreshWorkspace();
        if (!accountRequestIsCurrent()) return false;
        toast.success("Conversation deleted");
        return true;
      } catch (cause) {
        if (!accountRequestIsCurrent()) return false;
        toast.error(describeChatError(cause, "Could not delete that conversation."));
        return false;
      }
    },
    [accountRequestIsCurrent, chatAvailable, refreshWorkspace],
  );

  const openConversation = useCallback(
    async (targetId: ConversationId) => {
      if (!chatAvailable) return;
      if (getChatSnapshot().openingConversationId === targetId) return;
      const optimistic = beginConversationOpen(targetId, activeOpenRestore.current);
      if (!optimistic) return;
      leaveReservation(targetId);
      // A second selection while the first is pending must still roll back to
      // the last settled conversation, not to the first selection's skeleton.
      activeOpenRestore.current = optimistic;
      const request = ++openGeneration.current;
      setChatError(null);
      void chatConversationPreview(targetId)
        .then((preview) => {
          if (!preview || request !== openGeneration.current || !accountRequestIsCurrent()) return;
          applyConversationPreview(preview, targetId);
        })
        .catch((cause) => {
          // Preview is an optimization. The authoritative open below owns the
          // visible outcome, but a broken local read still belongs in logs.
          console.warn("Conversation preview unavailable", cause);
        });
      try {
        const view = await chatConversationOpen(targetId);
        if (request !== openGeneration.current || !accountRequestIsCurrent()) return;
        completeConversationOpen(view, targetId);
        activeOpenRestore.current = null;
        if (
          mcpSelectionRef.current.conversationId === targetId &&
          mcpSelectionRef.current.authGeneration === authGeneration &&
          mcpSelectionRef.current.status === "error"
        ) {
          setSelectionReloadRevision((revision) => revision + 1);
        }
      } catch (cause) {
        if (request !== openGeneration.current || !accountRequestIsCurrent()) return;
        // `not_found` is the backend's authoritative "the row is gone", as
        // distinct from "the network is gone". Self-heal the way xfalcon-web's
        // useMessageLoader does: tell the user, drop the dead row via a list
        // refresh, and reset the view only if the dead conversation was the one
        // on screen. Every other failure falls through to the inline error so
        // the row stays clickable for a retry.
        //
        // Read from the typed code, not from the message: matching prose by
        // substring broke the moment the copy was written for a reader.
        activeOpenRestore.current = null;
        rollbackConversationOpen(optimistic);
        if (chatErrorCode(cause) === "not_found") {
          toast.error("That conversation no longer exists — removing it from the list.");
          await refreshWorkspace();
          return;
        }
        // The command's reason is the only diagnostic surface this app has —
        // swallowing it leaves "doesn't work" with no cause to report.
        setChatError(describeChatError(cause, "Could not open that conversation."));
      }
    },
    [accountRequestIsCurrent, authGeneration, chatAvailable, leaveReservation, refreshWorkspace],
  );

  const editTurn = useCallback(
    async (messageId: string, text: string) => {
      if (!chatAvailable || !conversationId) return;
      let mcpServerId: McpServerId | null;
      try {
        mcpServerId = requireMcpSelection(conversationId);
      } catch (cause) {
        setChatError(cause instanceof Error ? cause.message : String(cause));
        return;
      }
      const trimmed = text.trim();
      if (!trimmed) return;

      const restore = beginTurnRevision(messageId);
      if (!restore) return;

      setChatError(null);

      try {
        const result = await chatMessageEdit(conversationId, messageId, trimmed, mcpServerId);
        if (!accountRequestIsCurrent()) return;
        if (result.status === "failed") {
          rollbackTurnRevision(restore);
          if (getChatSnapshot().conversationId !== conversationId) return;
          const message = result.message || "Could not edit message.";
          setChatError(message);
          toast.error(message);
          return;
        }
        if (result.conversation_id && result.run_id) {
          adoptActiveRun(result.conversation_id, result.run_id);
        }
        // Only close the edit box this edit itself opened. `turnEditStore` is
        // not conversation-scoped, so a reader who has since switched away and
        // started editing something else entirely -- another message, another
        // conversation -- is now the one showing through `activeMessageId`. An
        // unconditional clear here would erase that unrelated, still-in-progress
        // draft out from under them the moment this stale success lands.
        if (getChatSnapshot().conversationId === conversationId && getTurnEditState().activeMessageId === messageId) {
          clearTurnEdit();
        }
        finishTurnRevision(conversationId);
      } catch (cause) {
        if (!accountRequestIsCurrent()) return;
        rollbackTurnRevision(restore);
        if (getChatSnapshot().conversationId !== conversationId) return;
        const message = describeChatError(cause, "Could not edit message. The secure runtime did not respond.");
        setChatError(message);
        toast.error(message);
      }
    },
    [accountRequestIsCurrent, chatAvailable, conversationId, requireMcpSelection],
  );

  const regenerateTurn = useCallback(
    async (messageId: string) => {
      if (!chatAvailable || !conversationId) return;
      let mcpServerId: McpServerId | null;
      try {
        mcpServerId = requireMcpSelection(conversationId);
      } catch (cause) {
        setChatError(cause instanceof Error ? cause.message : String(cause));
        return;
      }

      const currentMessages = getChatSnapshot().messages;
      const targetIdx = currentMessages.findIndex((m) => m.id === messageId);
      if (targetIdx === -1) return;

      let userIdx = -1;
      for (let i = targetIdx; i >= 0; i--) {
        if (currentMessages[i].role === "user") {
          userIdx = i;
          break;
        }
      }
      if (userIdx === -1) return;

      const cutoffMsg = currentMessages[userIdx + 1];
      if (!cutoffMsg) {
        toast.error("No response to regenerate.");
        return;
      }
      const cutoffId = cutoffMsg.id;

      const restore = beginTurnRevision(cutoffId);
      if (!restore) return;

      clearTurnEdit();
      setChatError(null);

      try {
        const result = await chatMessageRegenerate(conversationId, messageId, mcpServerId);
        if (!accountRequestIsCurrent()) return;
        if (result.status === "failed") {
          rollbackTurnRevision(restore);
          if (getChatSnapshot().conversationId !== conversationId) return;
          const message = result.message || "Could not regenerate response.";
          setChatError(message);
          toast.error(message);
          return;
        }
        if (result.conversation_id && result.run_id) {
          adoptActiveRun(result.conversation_id, result.run_id);
        }
        finishTurnRevision(conversationId);
      } catch (cause) {
        if (!accountRequestIsCurrent()) return;
        rollbackTurnRevision(restore);
        if (getChatSnapshot().conversationId !== conversationId) return;
        const message = describeChatError(cause, "Could not regenerate response. The secure runtime did not respond.");
        setChatError(message);
        toast.error(message);
      }
    },
    [accountRequestIsCurrent, chatAvailable, conversationId, requireMcpSelection],
  );

  return {
    conversationId,
    mcpSelection:
      mcpSelection.conversationId === conversationId && mcpSelection.authGeneration === authGeneration
        ? mcpSelection
        : {
            conversationId,
            serverId: null,
            authGeneration,
            status: conversationId === null ? "ready" : "loading",
            error: null,
          },
    selectMcpServer,
    conversations,
    title,
    workspaceLoaded,
    /** The durable transcript, and ONLY that. The in-flight turn used to be
     * appended here, which built a fresh array on every render and so on every
     * streamed token — and a fresh array re-ran every memo keyed on it, from
     * `MessageList`'s whole item build down to `AppShell`'s file scan. The
     * transcript assembles that row itself now, from `useTranscript`
     * (spec: chat-streaming-performance). */
    messages,
    send,
    attachFile,
    pasteFiles,
    pastingFiles,
    attachingFile,
    cancelUpload,
    retryUpload,
    editTurn,
    regenerateTurn,
    cancelRun,
    retryLastRun,
    dismissNotice,
    startNewConversation,
    openConversation,
    pinConversation,
    deleteConversation,
    refreshWorkspace,
    pending: isBoundRunPending(activeRunId, runState),
    // Bound to the run on screen: a stale id left by a run that has since been
    // replaced must not draw the new turn's control as busy.
    stopping: stoppingRunId !== null && stoppingRunId === activeRunId,
  };
}
