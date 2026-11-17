import { Check, Copy, Pencil, RotateCcw } from "lucide-react";
import { memo, useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { Button } from "../../ui/Button";
import { IconButton } from "../../ui/IconButton";
import { cn } from "../../lib/cn";
import { writeClipboardText } from "./clipboard";
import { AssistantMessage } from "./AssistantMessage";
import { MessageFeedback } from "./MessageFeedback";
import { ResponseRibbon } from "./ResponseRibbon";
import { UserMessage } from "./UserMessage";
import type { ChatMessageRow } from "./chatStore";
import { REMEASURE_EVENT } from "./ToolActivityCard";
import {
  clearTurnEdit,
  startTurnEdit,
  updateTurnEditDraft,
  useTurnEdit,
  useTurnEditActiveMessageId,
} from "./turnEditStore";

export type MessageRowProps = {
  message: ChatMessageRow;
  /** This row is the newest assistant turn — it carries the brand-accent
   * rail (previous turns get the hairline) and, while its text is still the
   * synthetic in-flight stream row, the streaming caret. */
  isLatestAssistant?: boolean;
  /** Tokens are still arriving for this row. A synthetic live row can remain
   * mounted briefly after this becomes false while its durable replacement is
   * persisted. */
  streaming?: boolean;
  /**
   * This row opens the response, so it carries the boundary (spec:
   * chat-thread-layout). A multi-prose-row response carries several rows for one
   * turn — round-one prose above the tool umbrella, round-two prose below the
   * tool umbrella, then the answer below it — and the boundary marks the
   * handover from the user's turn, which happens once. Deciding it here from the
   * row alone is impossible: the caller is the only one that knows what came
   * before.
   */
  showRibbon?: boolean;
  /**
   * This row is the last prose row in its response, so it carries the regenerate
   * affordance (spec: chat-turn-revision).
   */
  isLastProse?: boolean;
  /** The conversation this turn belongs to. Carried only so an HTML document in
   * the answer can name its own source when saved to a project (spec: reports);
   * `null` before a conversation has landed. */
  conversationId?: string | null;
  onEdit?: (messageId: string, text: string) => void;
  onRegenerate?: (messageId: string) => void;
  revisionPending?: boolean;
};

function TurnEditor({
  originalText,
  onSave,
  onCancel,
}: {
  originalText: string;
  onSave: (text: string) => void;
  onCancel: () => void;
}) {
  const { draft } = useTurnEdit();
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const el = containerRef.current;
    const row = el?.closest<HTMLElement>("[data-index]") ?? el?.parentElement;
    el?.dispatchEvent(new CustomEvent(REMEASURE_EVENT, { bubbles: true }));
    textareaRef.current?.focus();
    if (textareaRef.current) {
      textareaRef.current.selectionStart = textareaRef.current.value.length;
      textareaRef.current.selectionEnd = textareaRef.current.value.length;
    }
    return () => {
      queueMicrotask(() => {
        row?.dispatchEvent(new CustomEvent(REMEASURE_EVENT, { bubbles: true }));
      });
    };
  }, []);

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onCancel();
      return;
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      const trimmed = draft.trim();
      if (!trimmed || trimmed === originalText.trim()) {
        onCancel();
      } else {
        onSave(trimmed);
      }
    }
  };

  const handleSubmit = () => {
    const trimmed = draft.trim();
    if (!trimmed || trimmed === originalText.trim()) {
      onCancel();
    } else {
      onSave(trimmed);
    }
  };

  const lineCount = draft.split("\n").length;

  return (
    <div
      ref={containerRef}
      className="w-full max-w-[90%] md:max-w-[80%] rounded-xl border border-default bg-card p-3 shadow-e1"
    >
      <textarea
        ref={textareaRef}
        aria-label="Edit message"
        className="w-full resize-none bg-transparent text-sm text-foreground placeholder:text-muted focus:outline-none"
        rows={Math.min(10, Math.max(2, lineCount))}
        value={draft}
        onChange={(e) => updateTurnEditDraft(e.target.value)}
        onKeyDown={handleKeyDown}
      />
      <div className="mt-2 flex items-center justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={onCancel} type="button">
          Cancel
        </Button>
        <Button
          size="sm"
          variant="primary"
          onClick={handleSubmit}
          disabled={!draft.trim() || draft.trim() === originalText.trim()}
          type="button"
        >
          Send
        </Button>
      </div>
    </div>
  );
}

/**
 * One conversation turn, styled like the Claude/ChatGPT reading surface:
 * the user's words sit in a distinct right-aligned bubble, the assistant's
 * answer is plain prose on the page marked by a 1px rail — no boxed cards,
 * no permanent role headers. Copy and response actions remain visible in a
 * reserved slot below the answer.
 */
export const MessageRow = memo(function MessageRow({
  message,
  isLatestAssistant = false,
  streaming = false,
  conversationId = null,
  showRibbon = true,
  isLastProse = false,
  onEdit,
  onRegenerate,
  revisionPending = false,
}: MessageRowProps) {
  const [copyState, setCopyState] = useState<"idle" | "copying" | "copied" | "failed">("idle");
  const activeTurnEditId = useTurnEditActiveMessageId();

  const copy = useCallback(async () => {
    setCopyState("copying");
    try {
      await writeClipboardText(message.text);
      setCopyState("copied");
      window.setTimeout(() => setCopyState("idle"), 1500);
    } catch {
      setCopyState("failed");
    }
  }, [message.text]);

  const isUser = message.role === "user";
  // The current answer row keeps a synthetic identity until its whole answer is
  // shown and its saved row has landed. Identity controls edit actions; the
  // explicit streaming signal controls the caret, which stays until the reveal
  // has shown the last character (spec: chat-motion-system).
  const isSynthetic = message.id.startsWith("stream-");
  const isStreamingRow = !isUser && streaming;
  const isOptimistic = message.id.startsWith("local-") || isSynthetic;
  const canEdit = isUser && !isOptimistic && onEdit !== undefined;
  const canRegenerate = !isUser && !isOptimistic && isLastProse && onRegenerate !== undefined;
  const isEditing = isUser && activeTurnEditId === message.id;
  const turnActionDisabled = revisionPending || activeTurnEditId !== null;

  const copyButton =
    copyState === "failed" ? (
      <Button size="sm" variant="danger" onClick={() => void copy()} type="button">
        Try copy again
      </Button>
    ) : (
      <IconButton
        aria-label={copyState === "copied" ? "Copied" : copyState === "copying" ? "Copying message" : "Copy message"}
        loading={copyState === "copying"}
        size="sm"
        tooltip={copyState === "copied" ? "Copied" : copyState === "copying" ? "Copying message" : "Copy message"}
        tooltipSide="top"
        type="button"
        onClick={() => void copy()}
      >
        {copyState === "copied" ? (
          <Check aria-hidden className="badge-pop text-success" size={14} />
        ) : (
          <Copy aria-hidden size={14} />
        )}
      </IconButton>
    );

  const editButton = canEdit ? (
    <IconButton
      aria-label="Edit message"
      className="actions-slide focus-visible:opacity-100"
      disabled={turnActionDisabled}
      size="sm"
      tooltip="Edit message"
      tooltipSide="top"
      type="button"
      onClick={() => startTurnEdit(message.id, message.text)}
    >
      <Pencil aria-hidden size={14} />
    </IconButton>
  ) : null;

  const regenerateButton = canRegenerate ? (
    <IconButton
      aria-label="Regenerate response"
      disabled={turnActionDisabled}
      size="sm"
      tooltip="Regenerate response"
      tooltipSide="top"
      type="button"
      onClick={() => onRegenerate?.(message.id)}
    >
      <RotateCcw aria-hidden size={14} />
    </IconButton>
  ) : null;

  if (isUser) {
    if (isEditing) {
      return (
        <article className="group/turn flex items-start justify-end gap-1 py-2.5">
          <TurnEditor
            originalText={message.text}
            onCancel={clearTurnEdit}
            onSave={(text) => onEdit?.(message.id, text)}
          />
        </article>
      );
    }

    return (
      <article className="group/turn flex items-start justify-end gap-1 py-2.5">
        <div className="user-bubble max-w-[90%] px-4 py-2.5 md:max-w-[80%]">
          <UserMessage attachments={message.attachments} text={message.text} />
        </div>
        <div className="flex flex-col gap-1">
          {copyButton}
          {editButton}
        </div>
      </article>
    );
  }

  return (
    <article className={cn("group/turn py-2.5", isLatestAssistant ? "assistant-accent-latest" : "assistant-accent")}>
      {message.text ? (
        <>
          {showRibbon ? <ResponseRibbon tone={isLatestAssistant ? "latest" : "history"} /> : null}
          <div className={cn(isStreamingRow && "streaming-caret")}>
            <AssistantMessage
              conversationId={conversationId}
              messageId={message.id}
              streaming={isStreamingRow}
              text={message.text}
            />
          </div>
        </>
      ) : null}
      <div className="mt-1 flex h-7 items-center gap-1">
        {copyButton}
        {regenerateButton}
        {!isOptimistic && isLastProse && conversationId ? (
          <MessageFeedback messageId={message.id} disabled={turnActionDisabled || streaming} />
        ) : null}
      </div>
    </article>
  );
});
