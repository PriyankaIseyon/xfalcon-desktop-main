import { useCallback, useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { Pencil, Pin, PinOff, Trash2 } from "lucide-react";
import type { ChatConversationSummary, ConversationId } from "../../lib/generated/contracts";
import { cn } from "../../lib/cn";
import { ContextMenu } from "../../ui/ContextMenu";
import { contextMenuAnchorFrom, isContextMenuKey, type ContextMenuAnchor } from "../../ui/menuContext";
import { Menu } from "../../ui/Menu";
import { TruncatedText } from "../../ui/TruncatedText";
import { groupConversations, RECENCY_SECTIONS, type RecencyBucket } from "./conversationOrder";
import { ConversationRowActions } from "./ConversationRowActions";

export type ConversationListProps = {
  conversations: ChatConversationSummary[];
  activeConversationId: ConversationId | null;
  disabled?: boolean;
  emptyMessage?: string;
  onSelect: (conversationId: ConversationId) => void;
  /** Pin or unpin one conversation. Omitted hides the pin action. */
  onPin?: (conversationId: ConversationId, pinned: boolean) => void;
  /** Permanently delete one conversation. Omitted hides the delete action. */
  onDelete?: (conversationId: ConversationId) => void;
  /** Give one conversation a new title. Omitted hides the rename action. */
  onRename?: (conversationId: ConversationId, title: string) => Promise<boolean>;
};

export function ConversationList({
  conversations,
  activeConversationId,
  disabled = false,
  emptyMessage = "No conversations yet. Start a new chat to begin.",
  onSelect,
  onPin,
  onDelete,
  onRename,
}: ConversationListProps) {
  const instanceId = useId();
  const pinnedHeadingId = `${instanceId}-pinned`;
  const recentsHeadingId = `${instanceId}-recents`;
  const itemRefs = useRef<HTMLButtonElement[]>([]);
  const renameInputRef = useRef<HTMLInputElement | null>(null);
  const [focusIndex, setFocusIndex] = useState(0);
  const [menu, setMenu] = useState<{ anchor: ContextMenuAnchor; conversation: ChatConversationSummary } | null>(null);
  /** The row being renamed in place, and what has been typed into it. */
  const [renaming, setRenaming] = useState<{ id: ConversationId; draft: string; saving: boolean } | null>(null);
  const groups = useMemo(() => groupConversations(conversations), [conversations]);
  const { pinned, recents, ordered } = groups;
  const recencySections = useMemo(
    () =>
      RECENCY_SECTIONS.map((section) => ({ ...section, rows: groups[section.key] })).filter(
        (section) => section.rows.length > 0,
      ),
    [groups],
  );
  const recencyStarts = useMemo(() => {
    let start = pinned.length;
    return recencySections.map((section) => {
      const current = start;
      start += section.rows.length;
      return current;
    });
  }, [pinned.length, recencySections]);
  const visibleOrdered = useMemo(() => [...pinned, ...recents], [pinned, recents]);

  useEffect(() => {
    const activeIndex = visibleOrdered.findIndex(
      (conversation) => conversation.conversation_id === activeConversationId,
    );
    if (activeIndex >= 0) {
      setFocusIndex(activeIndex);
    }
  }, [activeConversationId, visibleOrdered]);

  /**
   * Keeps the open conversation on screen.
   *
   * The list lives in a fixed-height band, so as chats accumulate the one you
   * are actually in drifts below the fold — and reopening the app, or picking a
   * chat from the command palette, would leave the sidebar showing a selection
   * you cannot see. `nearest` scrolls only when it is genuinely out of view,
   * so a visible selection never jumps.
   */
  useEffect(() => {
    if (activeConversationId == null) return;
    const index = visibleOrdered.findIndex((conversation) => conversation.conversation_id === activeConversationId);
    if (index < 0) return;
    // Optional call: jsdom does not implement it, and failing to scroll is not
    // a reason to fail a render.
    itemRefs.current[index]?.scrollIntoView?.({ block: "nearest" });
  }, [activeConversationId, visibleOrdered]);

  const focusItemAt = useCallback(
    (index: number) => {
      if (visibleOrdered.length === 0) return;
      const next = (index + visibleOrdered.length) % visibleOrdered.length;
      setFocusIndex(next);
      itemRefs.current[next]?.focus();
    },
    [visibleOrdered.length],
  );

  const onListKeyDown = (event: KeyboardEvent<HTMLUListElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "Home" && event.key !== "End") {
      return;
    }
    event.preventDefault();
    if (event.key === "Home") focusItemAt(0);
    else if (event.key === "End") focusItemAt(visibleOrdered.length - 1);
    else focusItemAt(focusIndex + (event.key === "ArrowDown" ? 1 : -1));
  };

  if (ordered.length === 0) {
    // The heading stays: it labels the band whether or not there is anything in
    // it yet, so the empty message still reads as the state of this list.
    return (
      <>
        <p className="sidebar-heading" id={recentsHeadingId}>
          <span>Recents</span>
        </p>
        <p className="type-chrome px-3 py-2 text-secondary">{emptyMessage}</p>
      </>
    );
  }

  itemRefs.current.length = visibleOrdered.length;

  const commitRename = async () => {
    if (renaming == null || renaming.saving) return;
    const next = renaming.draft.trim();
    const previous = ordered.find((item) => item.conversation_id === renaming.id)?.title ?? "";
    if (next.length === 0 || next === previous || onRename == null) {
      setRenaming(null);
      return;
    }

    const attempt = renaming;
    setRenaming({ ...attempt, saving: true });
    const succeeded = await onRename(attempt.id, next).catch(() => false);
    setRenaming((current) => {
      if (current?.id !== attempt.id || current.draft !== attempt.draft) return current;
      return succeeded ? null : { ...current, saving: false };
    });
    if (!succeeded) {
      queueMicrotask(() => {
        renameInputRef.current?.focus();
        renameInputRef.current?.select();
      });
    }
  };

  const renderRow = (conversation: ChatConversationSummary, index: number, bucket: RecencyBucket | "pinned") => {
    const active = conversation.conversation_id === activeConversationId;
    const title = conversation.title || "Untitled conversation";
    const pinnedRow = bucket === "pinned";

    // Renaming replaces the row rather than sitting beside it: the row is a
    // single click target, and a field competing with it inside the same
    // clickable box means every stray click opens the chat you are renaming.
    if (renaming?.id === conversation.conversation_id) {
      return (
        <li className="relative" key={conversation.conversation_id} role="presentation">
          <input
            ref={renameInputRef}
            autoFocus
            aria-label={`Rename ${title}`}
            aria-busy={renaming.saving || undefined}
            className="sidebar-row w-full border border-default bg-card text-left"
            disabled={renaming.saving}
            maxLength={120}
            value={renaming.draft}
            onBlur={commitRename}
            onChange={(event) => setRenaming({ id: renaming.id, draft: event.currentTarget.value, saving: false })}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                commitRename();
              }
              if (event.key === "Escape") {
                event.preventDefault();
                event.stopPropagation();
                setRenaming(null);
              }
            }}
          />
        </li>
      );
    }

    return (
      <li
        className="conversation-row-host relative"
        data-action-count={Number(Boolean(onPin)) + Number(Boolean(onDelete))}
        key={conversation.conversation_id}
        role="presentation"
      >
        <button
          ref={(node) => {
            if (node) itemRefs.current[index] = node;
          }}
          aria-current={active ? "page" : undefined}
          aria-label={title}
          aria-selected={active}
          className={cn("sidebar-row text-left", active && "active", pinnedRow && "sidebar-row--pinned")}
          disabled={disabled}
          role="option"
          tabIndex={index === focusIndex ? 0 : -1}
          type="button"
          onClick={() => onSelect(conversation.conversation_id)}
          onContextMenu={(event) => {
            event.preventDefault();
            setMenu({ anchor: contextMenuAnchorFrom(event, event.currentTarget), conversation });
          }}
          onFocus={() => setFocusIndex(index)}
          onKeyDown={(event) => {
            if (!isContextMenuKey(event)) return;
            event.preventDefault();
            setMenu({ anchor: contextMenuAnchorFrom(event, event.currentTarget), conversation });
          }}
        >
          <span className="sidebar-row__body">
            {pinnedRow ? <Pin aria-hidden className="sidebar-row__pin" size={11} strokeWidth={2} /> : null}
            <TruncatedText className="sidebar-row__title">{title}</TruncatedText>
          </span>
        </button>
        <ConversationRowActions conversation={conversation} disabled={disabled} onDelete={onDelete} onPin={onPin} />
      </li>
    );
  };

  /*
   * Every band is labelled here. A single "Recents" heading above mixed ages
   * would leave pinned chats and last week's work looking like one list.
   */
  return (
    <>
      {pinned.length > 0 ? (
        <div className="sidebar-group sidebar-group--pinned">
          <p className="sidebar-heading pb-0.5" id={pinnedHeadingId}>
            <span>Pinned</span>
            <span aria-hidden className="sidebar-heading__count">
              {pinned.length}
            </span>
          </p>
          <ul aria-labelledby={pinnedHeadingId} className="list-none p-0" role="listbox" onKeyDown={onListKeyDown}>
            {pinned.map((conversation, index) => renderRow(conversation, index, "pinned"))}
          </ul>
        </div>
      ) : null}
      {recencySections.map((section, sectionIndex) => {
        const headingId = `${instanceId}-${section.key}`;
        const startIndex = recencyStarts[sectionIndex] ?? pinned.length;
        return (
          <div className="sidebar-group" key={section.key}>
            <p className="sidebar-heading pb-0.5" id={headingId}>
              <span>{section.label}</span>
            </p>
            <ul aria-labelledby={headingId} className="list-none p-0" role="listbox" onKeyDown={onListKeyDown}>
              {section.rows.map((conversation, index) => renderRow(conversation, startIndex + index, section.key))}
            </ul>
          </div>
        );
      })}

      {/* The same two actions the hover buttons offer, but discoverable: right
          click, or Shift+F10 from the keyboard, on any row. */}
      <ContextMenu
        anchor={menu?.anchor ?? null}
        label={menu == null ? "" : `Actions for ${menu.conversation.title || "Untitled conversation"}`}
        onClose={() => setMenu(null)}
      >
        {menu == null ? null : (
          <>
            {onRename ? (
              <Menu.Item
                icon={<Pencil aria-hidden size={15} />}
                onSelect={() =>
                  setRenaming({
                    id: menu.conversation.conversation_id,
                    draft: menu.conversation.title || "Untitled conversation",
                    saving: false,
                  })
                }
              >
                Rename
              </Menu.Item>
            ) : null}
            {onPin ? (
              <Menu.Item
                icon={menu.conversation.pinned_at ? <PinOff aria-hidden size={15} /> : <Pin aria-hidden size={15} />}
                onSelect={() => onPin(menu.conversation.conversation_id, !menu.conversation.pinned_at)}
              >
                {menu.conversation.pinned_at ? "Unpin" : "Pin"}
              </Menu.Item>
            ) : null}
            {onDelete ? (
              <Menu.Item
                icon={<Trash2 aria-hidden size={15} />}
                onSelect={() => onDelete(menu.conversation.conversation_id)}
                variant="danger"
              >
                Delete
              </Menu.Item>
            ) : null}
          </>
        )}
      </ContextMenu>
    </>
  );
}
