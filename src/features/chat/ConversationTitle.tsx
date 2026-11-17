import { useEffect, useRef, useState } from "react";
import { Check, Pencil, X } from "lucide-react";
import { IconButton } from "../../ui/IconButton";
import { TruncatedText } from "../../ui/TruncatedText";

/** Web parity (`ChatHeader`): the same ceiling the web title field enforces. */
const TITLE_MAX = 120;

export interface ConversationTitleProps {
  title: string;
  subtitle?: string;
  /** `null` before the first send, when there is no row to rename yet. */
  conversationId: string | null;
  onRename: (title: string) => Promise<boolean>;
}

/**
 * The open conversation's name, renameable in place.
 *
 * Titles are written automatically from the first message, which means they
 * are frequently almost-right — "build a simple report using the ..." — and
 * until now there was no way to fix one. The backend has taken renames all
 * along (`chat_conversation_rename`); nothing in this app called it.
 *
 * Mirrors xfalcon-web's header: pencil to start, Enter or the tick to save,
 * Escape or the cross to abandon. Saving an empty or unchanged title is a
 * no-op rather than an error, because "I changed my mind" and "I typed
 * nothing" both mean leave it alone.
 */
export function ConversationTitle({ title, subtitle, conversationId, onRename }: ConversationTitleProps) {
  const [draft, setDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const titleRef = useRef<HTMLDivElement | null>(null);
  const attemptRef = useRef(0);
  const editing = draft !== null;

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  // A rename belongs to the conversation it started in. Switching away mid-edit
  // must not carry the draft onto the next chat and rename that one instead.
  useEffect(() => {
    attemptRef.current += 1;
    setDraft(null);
    setSaving(false);
  }, [conversationId]);

  const commit = async () => {
    if (saving) return;
    const next = draft?.trim() ?? "";
    if (next.length === 0 || next === title) {
      setDraft(null);
      return;
    }

    const attempt = ++attemptRef.current;
    setSaving(true);
    const succeeded = await onRename(next).catch(() => false);
    if (attemptRef.current !== attempt) return;
    if (succeeded) {
      setDraft(null);
      setSaving(false);
      return;
    }

    setSaving(false);
    queueMicrotask(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
  };

  return (
    <div
      className="conversation-title group/title"
      ref={titleRef}
      onBlur={(event) => {
        if (editing && !titleRef.current?.contains(event.relatedTarget)) void commit();
      }}
      onKeyDown={(event) => {
        if (editing && event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          attemptRef.current += 1;
          setDraft(null);
        }
      }}
    >
      <div className="conversation-title__text">
        {draft === null ? (
          <TruncatedText as="strong">{title}</TruncatedText>
        ) : (
          <input
            ref={inputRef}
            aria-label="Conversation title"
            aria-busy={saving || undefined}
            className="conversation-title__input rounded-md border border-default bg-card px-2 text-sm font-medium text-foreground"
            disabled={saving}
            maxLength={TITLE_MAX}
            value={draft}
            onChange={(event) => setDraft(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                commit();
              }
            }}
          />
        )}
        {subtitle ? <TruncatedText className="conversation-title__subtitle">{subtitle}</TruncatedText> : null}
      </div>
      {draft === null ? (
        conversationId == null ? null : (
          <IconButton
            aria-label="Rename conversation"
            className="opacity-0 transition-opacity focus-visible:opacity-100 group-hover/title:opacity-100"
            size="sm"
            onClick={() => setDraft(title)}
          >
            <Pencil aria-hidden size={13} />
          </IconButton>
        )
      ) : (
        <>
          <IconButton
            aria-label="Save title"
            disabled={saving}
            loading={saving}
            size="sm"
            // Keeps focus in the field so the click lands here rather than being
            // pre-empted by the blur commit, which would fire twice.
            onMouseDown={(event) => event.preventDefault()}
            onClick={commit}
          >
            <Check aria-hidden size={14} />
          </IconButton>
          <IconButton
            aria-label="Cancel rename"
            disabled={saving}
            size="sm"
            // Prevent a blur commit before the cancel click reaches this control.
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => {
              attemptRef.current += 1;
              setDraft(null);
            }}
          >
            <X aria-hidden size={14} />
          </IconButton>
        </>
      )}
    </div>
  );
}
