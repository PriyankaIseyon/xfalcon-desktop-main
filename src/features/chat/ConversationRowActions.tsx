import { Pin, PinOff, Trash2 } from "lucide-react";
import type { ChatConversationSummary, ConversationId } from "../../lib/generated/contracts";
import { IconButton } from "../../ui/IconButton";

export interface ConversationRowActionsProps {
  conversation: ChatConversationSummary;
  disabled: boolean;
  /** Pin or unpin one conversation. Omitted hides the pin action. */
  onPin?: (conversationId: ConversationId, pinned: boolean) => void;
  /** Permanently delete one conversation. Omitted hides the delete action. */
  onDelete?: (conversationId: ConversationId) => void;
}

/**
 * Pin and delete for one conversation row.
 *
 * Shared because a conversation is listed in two places now — flat under
 * Recents and nested under its project — and two copies of the same two
 * actions is two chances for them to drift apart. Both use the same hover
 * positioning and keyboard focus reveal.
 *
 * Built on the same `IconButton` every other icon-only action in the app
 * uses (`ReportActions`, `Pagination`, `Modal`'s close control) rather than a
 * hand-rolled button, sized `sm` for the compact sidebar row.
 * Delete carries `variant="danger"` for the same reason `ReportActions`' does —
 * one hover color for "this deletes something", everywhere it appears.
 *
 * Clicks stop here: in the tree these buttons sit inside a row that opens the
 * conversation, and pinning a chat must not also switch to it.
 */
export function ConversationRowActions({ conversation, disabled, onPin, onDelete }: ConversationRowActionsProps) {
  const title = conversation.title || "Untitled conversation";
  const pinned = Boolean(conversation.pinned_at);
  return (
    <span className="conversation-row-actions">
      {onPin ? (
        <IconButton
          aria-label={pinned ? `Unpin ${title}` : `Pin ${title}`}
          disabled={disabled}
          size="sm"
          onClick={(event) => {
            event.stopPropagation();
            onPin(conversation.conversation_id, !pinned);
          }}
        >
          {pinned ? <PinOff aria-hidden size={14} /> : <Pin aria-hidden size={14} />}
        </IconButton>
      ) : null}
      {onDelete ? (
        <IconButton
          aria-label={`Delete ${title}`}
          disabled={disabled}
          size="sm"
          variant="danger"
          onClick={(event) => {
            event.stopPropagation();
            onDelete(conversation.conversation_id);
          }}
        >
          <Trash2 aria-hidden size={14} />
        </IconButton>
      ) : null}
    </span>
  );
}
