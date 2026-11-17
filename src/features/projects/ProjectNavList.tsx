import React from "react";
import {
  ChevronRight,
  ChevronsDown,
  ChevronsUp,
  File,
  FolderOpen,
  FolderPlus,
  FolderTree,
  MessageSquarePlus,
  Pencil,
  Pin,
  PinOff,
  ShieldAlert,
  Trash2,
} from "lucide-react";
import { cn } from "../../lib/cn";
import type { ChatConversationSummary, ConversationId, ProjectView } from "../../lib/generated/contracts";
import { AlertDialog } from "../../ui/AlertDialog";
import { ContextMenu } from "../../ui/ContextMenu";
import { contextMenuAnchorFrom, isContextMenuKey, type ContextMenuAnchor } from "../../ui/menuContext";
import { IconButton } from "../../ui/IconButton";
import { Menu } from "../../ui/Menu";
import { Tooltip } from "../../ui/Tooltip";
import { TruncatedText } from "../../ui/TruncatedText";
import { groupConversations } from "../chat/conversationOrder";
import { ConversationRowActions } from "../chat/ConversationRowActions";
import { ProjectCreateSheet } from "./ProjectCreateSheet";
import { describeProjectError } from "./projectFileText";
import { ProjectStatus, type ProjectIndexStatus } from "./ProjectStatus";
import { NO_CHAT_FILES, type ChatFile, type ChatFileList } from "./chatFiles";

export interface ProjectNavListProps {
  /** The sidebar's "+" asked for a new project: open the create sheet once. */
  createRequested?: boolean;
  /** Called once the create sheet has been opened for that request. */
  onCreateHandled?: () => void;
  projects: readonly ProjectView[];
  activeProjectId?: string;
  /** That project's conversations, newest first. */
  conversationsFor: (projectId: string) => ChatConversationSummary[];
  activeConversationId: ConversationId | null;
  /**
   * The files the OPEN conversation used. Only the open one: a summary row
   * carries no tool steps, so this is the only conversation whose files the
   * sidebar can name without asking the backend for every chat at once.
   */
  activeConversationFiles: ChatFileList;
  /** Mirrors the sidebar's own gate: no chat action works before the backend is ready. */
  chatReady: boolean;
  /**
   * What the Projects destination is showing beside this list: the project's
   * files, or one of its conversations. It decides which row reads as the one
   * you have open -- without it a chat and a file could both claim it, or
   * neither could.
   */
  pane: "files" | "chat";
  /** The path the file view has open, or `null` for none. */
  openFilePath: string | null;
  /**
   * Withdraws trust from one project: it leaves the sidebar and xFalcon stops
   * reaching into it. Named "remove" rather than "delete" throughout because
   * the backend (`project_revoke`) drops the trust record and nothing else --
   * the folder and every file in it stay exactly where they are.
   */
  onRemoveProject: (projectId: string) => Promise<void>;
  projectStatus?: Readonly<Record<string, ProjectIndexStatus>>;
  /**
   * Starts a NEW chat in one project, shown beside this tree the same way the
   * project's existing chats are. Every project row carries it: a container
   * you can only fill through a context menu has no visible way to be filled,
   * and this is the one action a project row exists to offer.
   */
  onStartProjectChat: (projectId: string) => void;
  onBrowseFiles: (projectId: string) => void;
  onOpenFile: (projectId: string, path: string) => void;
  onOpenConversation: (conversationId: ConversationId) => void;
  onPinConversation: (conversationId: ConversationId, pinned: boolean) => void;
  onDeleteConversation: (conversationId: ConversationId) => void;
  /**
   * Give one of a project's chats a new title. Resolves to whether it
   * succeeded, so the row's own in-place editor knows whether to close or
   * to keep the typed draft open for a retry.
   */
  onRenameConversation: (conversationId: ConversationId, title: string) => Promise<boolean>;
  onProjectReady: (project: ProjectView) => void;
}

/**
 * One row of the projects tree, flattened to what is visible right now.
 *
 * Flat rather than nested because that is what the keyboard needs: Up and Down
 * move between adjacent VISIBLE rows regardless of which parent each belongs
 * to, and a nested render would have to walk the tree to answer "what is the
 * next row". Depth is carried per row instead, and the same number drives the
 * indent, the guide rails and `aria-level`, so the three can never disagree.
 */
type NavRow = { key: string; depth: number; expandable: boolean; expanded: boolean } & (
  | { kind: "project"; project: ProjectView }
  | { kind: "conversation"; projectId: string; conversation: ChatConversationSummary }
  | { kind: "file"; projectId: string; file: ChatFile }
  | { kind: "browse"; projectId: string; projectName: string }
  | { kind: "note"; text: string }
);

/** Index of the row that owns `rows[index]`, or -1 at the top level. */
function parentIndexOf(rows: readonly NavRow[], index: number): number {
  const depth = rows[index]?.depth ?? 0;
  for (let candidate = index - 1; candidate >= 0; candidate -= 1) {
    if (rows[candidate].depth < depth) return candidate;
  }
  return -1;
}

function conversationTitle(conversation: ChatConversationSummary): string {
  return conversation.title || "Untitled conversation";
}

/** What a row calls itself, for naming its context menu. */
function rowLabel(row: NavRow): string {
  switch (row.kind) {
    case "project":
      return row.project.display_name;
    case "conversation":
      return conversationTitle(row.conversation);
    case "file":
      return fileName(row.file.path);
    case "browse":
      return `browse files in ${row.projectName}`;
    case "note":
      return row.text;
  }
}

/** The file name, which is what a person identifies a path by. */
function fileName(path: string): string {
  const cut = path.lastIndexOf("/");
  return cut === -1 ? path : path.slice(cut + 1);
}

/**
 * Every project, openable to reveal what it contains: its chats, each chat's
 * own files, and the project's whole file tree one row further down.
 *
 * It is a tree, not a list of lists. Four levels of belonging (project → chat →
 * file) need a reader to see at a glance what sits under what, and a source
 * list only carries two. Indentation, disclosure, icons, and `aria-level` all
 * read from the same depth number.
 *
 * Selecting and expanding stay separate actions, as they were. The name
 * activates the project (and opens it, because the reason you clicked it is to
 * look inside); the chevron only expands, so you can read one project's
 * contents without switching away from the one you are working in. On the
 * keyboard that split is Enter versus the arrow keys.
 *
 * Focus stays on the tree container and the active row is named by
 * `aria-activedescendant`, matching `ProjectTree` — one keyboard model for both
 * trees in this app.
 */
export function ProjectNavList({
  createRequested = false,
  onCreateHandled,
  projects,
  activeProjectId,
  conversationsFor,
  activeConversationId,
  activeConversationFiles,
  chatReady,
  pane,
  openFilePath,
  projectStatus,
  onStartProjectChat,
  onBrowseFiles,
  onOpenFile,
  onOpenConversation,
  onPinConversation,
  onDeleteConversation,
  onRenameConversation,
  onProjectReady,
  onRemoveProject,
}: ProjectNavListProps) {
  const treeId = React.useId();
  const treeRef = React.useRef<HTMLDivElement | null>(null);
  const [createOpen, setCreateOpen] = React.useState(false);
  React.useEffect(() => {
    if (!createRequested) return;
    setCreateOpen(true);
    onCreateHandled?.();
  }, [createRequested, onCreateHandled]);
  /**
   * The project whose removal is awaiting confirmation. Held as the project
   * itself, not its id, so the dialog can name it -- a destructive prompt that
   * says "this project" makes the reader verify which row they clicked.
   */
  const [pendingRemoval, setPendingRemoval] = React.useState<ProjectView | null>(null);
  const [removing, setRemoving] = React.useState(false);
  const [removalError, setRemovalError] = React.useState<string | null>(null);
  const removalRequestRef = React.useRef(0);
  /** The row whose context menu is open, and where it was summoned. */
  const [menu, setMenu] = React.useState<{ anchor: ContextMenuAnchor; row: NavRow } | null>(null);
  /** The chat row being renamed in place, and what has been typed into it. */
  const [renamingChat, setRenamingChat] = React.useState<{ id: ConversationId; draft: string } | null>(null);
  /**
   * Guards a rename commit against a stale close: bumped whenever renaming
   * starts, is cancelled, or moves to a different row, so a slow commit for
   * an attempt the person already left cannot later close (or reopen) an
   * editor that has moved on to something else.
   */
  const renameRequestRef = React.useRef(0);
  const [activeIndex, setActiveIndex] = React.useState(0);
  /**
   * Projects the person has opened or closed by hand. Anything absent falls
   * back to "open if it is the active project", so activating a project
   * elsewhere (the command palette, opening one of its chats) reveals it here
   * without the sidebar ever claiming the current project is collapsed.
   *
   * A decision, not a set of open ids, because the fallback and an explicit
   * choice would otherwise fight: collapsing the active project would be undone
   * on the next render that re-applied the fallback.
   */
  const [projectDecisions, setProjectDecisions] = React.useState<ReadonlyMap<string, boolean>>(() => new Map());
  const [closedChats, setClosedChats] = React.useState<ReadonlySet<string>>(() => new Set());

  React.useEffect(
    () => () => {
      removalRequestRef.current += 1;
    },
    [],
  );

  const requestRemoval = React.useCallback((project: ProjectView) => {
    removalRequestRef.current += 1;
    setPendingRemoval(project);
    setRemoving(false);
    setRemovalError(null);
  }, []);

  const cancelRemoval = React.useCallback(() => {
    if (removing) return;
    removalRequestRef.current += 1;
    setPendingRemoval(null);
    setRemovalError(null);
  }, [removing]);

  const confirmRemoval = React.useCallback(async () => {
    if (pendingRemoval == null || removing) return;
    const request = ++removalRequestRef.current;
    setRemoving(true);
    setRemovalError(null);
    try {
      await onRemoveProject(pendingRemoval.id);
      if (request === removalRequestRef.current) setPendingRemoval(null);
    } catch (cause: unknown) {
      if (request === removalRequestRef.current) {
        setRemovalError(describeProjectError(cause, "This project could not be removed."));
      }
    } finally {
      if (request === removalRequestRef.current) setRemoving(false);
    }
  }, [onRemoveProject, pendingRemoval, removing]);

  const isProjectOpen = React.useCallback(
    (projectId: string) => projectDecisions.get(projectId) ?? projectId === activeProjectId,
    [activeProjectId, projectDecisions],
  );

  const toggleProject = React.useCallback(
    (projectId: string) => {
      setProjectDecisions((current) => new Map(current).set(projectId, !isProjectOpen(projectId)));
    },
    [isProjectOpen],
  );
  const allProjectsOpen = projects.length > 0 && projects.every((project) => isProjectOpen(project.id));
  const setAllProjectsOpen = React.useCallback(
    (open: boolean) => {
      setProjectDecisions(new Map(projects.map((project) => [project.id, open])));
    },
    [projects],
  );

  // Chats are open by default and remembered when closed, the opposite of
  // projects. Only the open chat has files to show, and that chat is where the
  // person is working -- collapsing it by default would hide the one branch
  // this tree exists to show.
  const toggleChat = React.useCallback((conversationId: string) => {
    setClosedChats((current) => {
      const next = new Set(current);
      if (!next.delete(conversationId)) next.add(conversationId);
      return next;
    });
  }, []);

  const rows = React.useMemo<NavRow[]>(() => {
    const built: NavRow[] = [];
    for (const project of projects) {
      const open = isProjectOpen(project.id);
      built.push({
        key: `project:${project.id}`,
        depth: 0,
        kind: "project",
        project,
        expandable: true,
        expanded: open,
      });
      if (!open) continue;

      const conversations = groupConversations(conversationsFor(project.id)).ordered;
      if (conversations.length === 0) {
        built.push({
          key: `no-chats:${project.id}`,
          depth: 1,
          kind: "note",
          text: "No chats in this project yet.",
          expandable: false,
          expanded: false,
        });
      }
      for (const conversation of conversations) {
        const id = conversation.conversation_id;
        const files = id === activeConversationId ? activeConversationFiles : NO_CHAT_FILES;
        const expandable = files.files.length > 0;
        const expanded = expandable && !closedChats.has(id);
        built.push({
          key: `chat:${id}`,
          depth: 1,
          kind: "conversation",
          projectId: project.id,
          conversation,
          expandable,
          expanded,
        });
        if (!expanded) continue;
        for (const file of files.files) {
          built.push({
            key: `file:${id}:${file.path}`,
            depth: 2,
            kind: "file",
            projectId: project.id,
            file,
            expandable: false,
            expanded: false,
          });
        }
        if (files.hidden > 0) {
          built.push({
            key: `more:${id}`,
            depth: 2,
            kind: "note",
            text: `${files.hidden} more file${files.hidden === 1 ? "" : "s"} — browse files`,
            expandable: false,
            expanded: false,
          });
        }
      }
      built.push({
        key: `browse:${project.id}`,
        depth: 1,
        kind: "browse",
        projectId: project.id,
        projectName: project.display_name,
        expandable: false,
        expanded: false,
      });
    }
    return built;
  }, [activeConversationFiles, activeConversationId, closedChats, conversationsFor, isProjectOpen, projects]);

  /**
   * Whether the open file is one this tree draws a row for. A file opened from
   * the project's own file view usually is not: only the open chat's files get
   * rows. When it has no row of its own, the project's file row is what stands
   * for it -- otherwise a file could be open with nothing in the sidebar
   * marked, and the reader would have no idea where they are.
   */
  const openFileHasRow =
    openFilePath != null && activeConversationFiles.files.some((file) => file.path === openFilePath);

  // Rows collapse and disappear, so a stored index can outlive its row.
  const boundedIndex = rows.length === 0 ? -1 : Math.min(activeIndex, rows.length - 1);
  const activeRow = boundedIndex === -1 ? undefined : rows[boundedIndex];

  const activate = React.useCallback(
    (row: NavRow | undefined) => {
      if (!row) return;
      switch (row.kind) {
        case "project":
          // A container opens and closes. It used to ALSO start a new chat in
          // the project and leave for the chat surface, which threw away the
          // conversation that was open and swapped the column out from under
          // the reader -- for a press whose obvious meaning is "show me what is
          // in here".
          toggleProject(row.project.id);
          return;
        case "conversation":
          if (chatReady) onOpenConversation(row.conversation.conversation_id);
          return;
        case "file":
          onOpenFile(row.projectId, row.file.path);
          return;
        case "browse":
          onBrowseFiles(row.projectId);
          return;
        case "note":
          return;
      }
    },
    [chatReady, onBrowseFiles, onOpenConversation, onOpenFile, toggleProject],
  );

  const toggle = React.useCallback(
    (row: NavRow) => {
      if (row.kind === "project") toggleProject(row.project.id);
      else if (row.kind === "conversation") toggleChat(row.conversation.conversation_id);
    },
    [toggleChat, toggleProject],
  );

  /**
   * Keeps whatever is open visible inside a fixed-height band. As projects and
   * their chats accumulate the selected row drifts past the fold, and a
   * selection you cannot see is the same as no selection at all. `nearest`
   * means a row already on screen never moves.
   */
  const currentRowKey = rows.find((row) =>
    pane === "chat"
      ? row.kind === "conversation" && row.conversation.conversation_id === activeConversationId
      : row.kind === "file" && row.file.path === openFilePath,
  )?.key;
  React.useEffect(() => {
    if (currentRowKey == null) return;
    const row = treeRef.current?.querySelector<HTMLElement>(`#${CSS.escape(`${treeId}-${currentRowKey}`)}`);
    // Optional because jsdom does not implement it, and a sidebar that cannot
    // scroll a row into view is not a reason to fail a render.
    row?.scrollIntoView?.({ block: "nearest" });
  }, [currentRowKey, treeId]);

  /**
   * What one row offers, as a function of the row.
   *
   * A function rather than a chain of ternaries on `menu.row`: narrowing a
   * property of a state object does not survive into the callbacks below it,
   * so every `menu.row.project` there was an unchecked assumption. A parameter
   * narrows properly, and the switch is exhaustive over `NavRow["kind"]`.
   */
  const rowMenuItems = (row: NavRow): React.ReactNode => {
    switch (row.kind) {
      case "project":
        return (
          <>
            <Menu.Item icon={<FolderTree aria-hidden size={15} />} onSelect={() => onBrowseFiles(row.project.id)}>
              Show all files
            </Menu.Item>
            {/* The keyboard's route to the button on the row itself. Gated
                like every other chat action here: nothing in this column can
                start a conversation before the backend can hold one. */}
            <Menu.Item
              disabled={!chatReady}
              icon={<MessageSquarePlus aria-hidden size={15} />}
              onSelect={() => onStartProjectChat(row.project.id)}
            >
              New chat in this project
            </Menu.Item>
            <Menu.Separator />
            <Menu.Item
              icon={<Trash2 aria-hidden size={15} />}
              onSelect={() => requestRemoval(row.project)}
              variant="danger"
            >
              Remove project…
            </Menu.Item>
          </>
        );
      case "conversation":
        return (
          <>
            <Menu.Item
              disabled={!chatReady}
              icon={<Pencil aria-hidden size={15} />}
              onSelect={() => {
                renameRequestRef.current += 1;
                setRenamingChat({ id: row.conversation.conversation_id, draft: rowLabel(row) });
              }}
            >
              Rename
            </Menu.Item>
            <Menu.Item
              disabled={!chatReady}
              icon={row.conversation.pinned_at ? <PinOff aria-hidden size={15} /> : <Pin aria-hidden size={15} />}
              onSelect={() => onPinConversation(row.conversation.conversation_id, !row.conversation.pinned_at)}
            >
              {row.conversation.pinned_at ? "Unpin" : "Pin"}
            </Menu.Item>
            <Menu.Item
              disabled={!chatReady}
              icon={<Trash2 aria-hidden size={15} />}
              onSelect={() => onDeleteConversation(row.conversation.conversation_id)}
              variant="danger"
            >
              Delete chat
            </Menu.Item>
          </>
        );
      case "file":
        return (
          <Menu.Item icon={<File aria-hidden size={15} />} onSelect={() => onOpenFile(row.projectId, row.file.path)}>
            Open file
          </Menu.Item>
        );
      case "browse":
        return (
          <Menu.Item icon={<FolderTree aria-hidden size={15} />} onSelect={() => onBrowseFiles(row.projectId)}>
            Show all files
          </Menu.Item>
        );
      case "note":
        return null;
    }
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (rows.length === 0) return;
    const moveTo = (index: number) => {
      event.preventDefault();
      setActiveIndex(Math.max(0, Math.min(index, rows.length - 1)));
    };

    // Focus sits on the tree, not the row, so the cursor row has to be found
    // by id to anchor its menu -- without this, a keyboard user has no route
    // to any row action at all.
    if (isContextMenuKey(event) && activeRow != null && activeRow.kind !== "note") {
      event.preventDefault();
      const cursor = treeRef.current?.querySelector<HTMLElement>(`#${CSS.escape(`${treeId}-${activeRow.key}`)}`);
      setMenu({ anchor: contextMenuAnchorFrom(event, cursor ?? treeRef.current), row: activeRow });
      return;
    }

    switch (event.key) {
      case "ArrowDown":
        moveTo(boundedIndex + 1);
        return;
      case "ArrowUp":
        moveTo(boundedIndex - 1);
        return;
      case "Home":
        moveTo(0);
        return;
      case "End":
        moveTo(rows.length - 1);
        return;
      case "ArrowRight":
        if (activeRow?.expandable && !activeRow.expanded) {
          event.preventDefault();
          toggle(activeRow);
        } else {
          moveTo(boundedIndex + 1);
        }
        return;
      case "ArrowLeft":
        if (activeRow?.expandable && activeRow.expanded) {
          event.preventDefault();
          toggle(activeRow);
          return;
        }
        moveTo(parentIndexOf(rows, boundedIndex));
        return;
      case "Enter":
      case " ":
        event.preventDefault();
        activate(activeRow);
        return;
      default:
        return;
    }
  };

  return (
    <>
      {/* The destination above this list is already called Projects, so the
          column does not say it a second time. What the row carries instead is
          the one action that fills the list -- the peer of New chat on the
          Chats destination -- with the tree's own density control beside it. */}
      <div className="app-sidebar__heading">
        <button
          className="nav-item nav-item--child nav-item--create w-full min-w-0 border-0 bg-transparent"
          type="button"
          onClick={() => setCreateOpen(true)}
        >
          <FolderPlus aria-hidden size={16} strokeWidth={1.8} />
          <span>New project</span>
        </button>
        {projects.length > 0 ? (
          <IconButton
            aria-label={allProjectsOpen ? "Collapse all projects" : "Expand all projects"}
            size="sm"
            onClick={() => setAllProjectsOpen(!allProjectsOpen)}
          >
            {allProjectsOpen ? <ChevronsUp aria-hidden size={14} /> : <ChevronsDown aria-hidden size={14} />}
          </IconButton>
        ) : null}
      </div>

      {projects.length === 0 ? (
        <p className="type-chrome px-3 py-2 text-faint">No projects yet.</p>
      ) : (
        <div className="app-sidebar__scroll" id={`${treeId}-panel`}>
          <div
            aria-activedescendant={activeRow ? `${treeId}-${activeRow.key}` : undefined}
            aria-label="Projects"
            className="sidebar-tree"
            onKeyDown={handleKeyDown}
            ref={treeRef}
            role="tree"
            tabIndex={0}
          >
            {rows.map((row, index) => (
              <NavTreeRow
                activeConversationId={activeConversationId}
                activeProjectId={activeProjectId}
                chatReady={chatReady}
                id={`${treeId}-${row.key}`}
                pane={pane}
                isCursor={row === activeRow}
                key={row.key}
                openFileHasRow={openFileHasRow}
                openFilePath={openFilePath}
                projectStatus={projectStatus}
                row={row}
                onActivate={() => {
                  setActiveIndex(index);
                  // Focus lives on the tree, not the row, so a mouse click has to
                  // move it there or the arrow keys would go nowhere afterwards.
                  treeRef.current?.focus();
                  activate(row);
                }}
                renaming={renamingChat}
                onRenameCancel={() => {
                  renameRequestRef.current += 1;
                  setRenamingChat(null);
                }}
                onRenameChange={(draft) =>
                  setRenamingChat((current) => (current == null ? null : { ...current, draft }))
                }
                onRenameCommit={() => {
                  if (renamingChat == null) return;
                  const next = renamingChat.draft.trim();
                  if (next.length === 0) {
                    renameRequestRef.current += 1;
                    setRenamingChat(null);
                    return;
                  }
                  const attemptId = renamingChat.id;
                  const request = ++renameRequestRef.current;
                  // Closing only on confirmed success -- a rejected rename
                  // already announces itself through a toast, and the row
                  // stays open on the exact title the person typed instead
                  // of discarding it under an optimistic close.
                  void onRenameConversation(attemptId, next).then((succeeded) => {
                    if (succeeded && request === renameRequestRef.current) setRenamingChat(null);
                  });
                }}
                onContextMenu={(anchor) => {
                  setActiveIndex(index);
                  setMenu({ anchor, row });
                }}
                onDeleteConversation={onDeleteConversation}
                onPinConversation={onPinConversation}
                onRemoveProject={requestRemoval}
                onStartProjectChat={onStartProjectChat}
                onToggle={() => {
                  setActiveIndex(index);
                  toggle(row);
                }}
              />
            ))}
          </div>
        </div>
      )}

      {/* What each kind of row can do, spelled out. The hover buttons stay for
          the one action each row uses most; this is where the rest lives, and
          it is the only route to any of them from the keyboard. */}
      <ContextMenu
        anchor={menu?.anchor ?? null}
        label={menu == null ? "" : `Actions for ${rowLabel(menu.row)}`}
        onClose={() => setMenu(null)}
      >
        {menu == null ? null : rowMenuItems(menu.row)}
      </ContextMenu>

      <ProjectCreateSheet
        onClose={() => setCreateOpen(false)}
        onReady={(project) => {
          onProjectReady(project);
          setCreateOpen(false);
        }}
        open={createOpen}
      />

      {/* Removing a project is the one irreversible thing this tree can do, so
          it is confirmed -- and the description says outright what survives.
          "Delete" would be the wrong word on this button: nothing on disk is
          touched, and a person who reads it as "delete my folder" and clicks
          anyway has been misled either way round. */}
      <AlertDialog
        confirmLabel="Remove project"
        description={
          pendingRemoval == null
            ? ""
            : `xFalcon will stop showing ${pendingRemoval.display_name} and lose access to its files. The folder and every file in it stay on this computer. Existing chats are not deleted. You can add the folder again later.`
        }
        destructive
        busy={removing}
        error={removalError}
        onCancel={cancelRemoval}
        onConfirm={() => void confirmRemoval()}
        open={pendingRemoval != null}
        title="Remove this project?"
      />
    </>
  );
}

interface NavTreeRowProps {
  row: NavRow;
  id: string;
  isCursor: boolean;
  activeProjectId?: string;
  activeConversationId: ConversationId | null;
  pane: "files" | "chat";
  openFilePath: string | null;
  /** Whether the open file is drawn as a row of its own somewhere in the tree. */
  openFileHasRow: boolean;
  chatReady: boolean;
  projectStatus?: Readonly<Record<string, ProjectIndexStatus>>;
  onActivate: () => void;
  onToggle: () => void;
  onPinConversation: (conversationId: ConversationId, pinned: boolean) => void;
  onDeleteConversation: (conversationId: ConversationId) => void;
  onRemoveProject: (project: ProjectView) => void;
  onStartProjectChat: (projectId: string) => void;
  onContextMenu: (anchor: ContextMenuAnchor) => void;
  /** The chat being renamed anywhere in the tree, or `null`. */
  renaming: { id: ConversationId; draft: string } | null;
  onRenameChange: (draft: string) => void;
  onRenameCommit: () => void;
  onRenameCancel: () => void;
}

/** One rendered row. Indent spacers keep every depth on the shared label grid. */
function NavTreeRow({
  row,
  id,
  isCursor,
  activeProjectId,
  activeConversationId,
  pane,
  openFilePath,
  openFileHasRow,
  chatReady,
  projectStatus,
  onActivate,
  onToggle,
  onPinConversation,
  onDeleteConversation,
  onRemoveProject,
  onStartProjectChat,
  onContextMenu,
  renaming,
  onRenameChange,
  onRenameCommit,
  onRenameCancel,
}: NavTreeRowProps) {
  const belongsToActiveProject = row.kind === "note" ? false : "projectId" in row && row.projectId === activeProjectId;
  const activeProjectContext = row.kind === "project" && row.project.id === activeProjectId;
  /* One mark, and it follows what the destination is actually showing beside
     this list. With a project's chat on screen the chat row is the open one;
     with its files on screen a file row is. Two separate booleans used to
     answer this and could both say yes at once. */
  const current =
    pane === "chat"
      ? row.kind === "conversation" && row.conversation.conversation_id === activeConversationId
      : row.kind === "file"
        ? belongsToActiveProject && row.file.path === openFilePath
        : row.kind === "browse"
          ? belongsToActiveProject && !openFileHasRow
          : false;
  const inert = row.kind === "note" || (row.kind === "conversation" && !chatReady);

  return (
    <div
      aria-current={current ? "page" : undefined}
      aria-disabled={inert || undefined}
      aria-expanded={row.expandable ? row.expanded : undefined}
      aria-level={row.depth + 1}
      aria-selected={row.kind === "note" ? undefined : current}
      className={cn(
        "sidebar-row",
        row.kind === "conversation" && "conversation-row-host",
        row.kind === "project" && "sidebar-row--project",
        row.kind === "project" && projectStatus?.[row.project.id] && "sidebar-row--with-status",
        row.kind === "note" && "sidebar-row--note",
        row.kind === "browse" && "sidebar-row--browse",
        current && "active",
        activeProjectContext && "sidebar-row--project-context",
        isCursor && "sidebar-row--cursor",
      )}
      id={id}
      data-action-count={row.kind === "conversation" ? 2 : undefined}
      onClick={inert ? undefined : onActivate}
      onContextMenu={
        row.kind === "note"
          ? undefined
          : (event) => {
              event.preventDefault();
              onContextMenu(contextMenuAnchorFrom(event, event.currentTarget));
            }
      }
      role="treeitem"
    >
      {row.expandable ? (
        <button
          aria-hidden
          className="sidebar-twisty"
          tabIndex={-1}
          type="button"
          onClick={(event) => {
            // The row underneath opens the project; this only reveals it.
            event.stopPropagation();
            onToggle();
          }}
        >
          <ChevronRight
            aria-hidden
            className={cn("transition-transform duration-fast", row.expanded && "rotate-90")}
            size={14}
            strokeWidth={2.2}
          />
        </button>
      ) : (
        <span aria-hidden className="sidebar-twisty" />
      )}

      {row.kind === "conversation" && renaming?.id === row.conversation.conversation_id ? (
        <input
          autoFocus
          aria-label={`Rename ${conversationTitle(row.conversation)}`}
          className="min-w-0 flex-1 rounded-sm border border-default bg-card px-1"
          maxLength={120}
          value={renaming.draft}
          onBlur={onRenameCommit}
          onChange={(event) => onRenameChange(event.currentTarget.value)}
          // The row underneath opens the chat; typing in here must not.
          onClick={(event) => event.stopPropagation()}
          onKeyDown={(event) => {
            event.stopPropagation();
            if (event.key === "Enter") {
              event.preventDefault();
              onRenameCommit();
            }
            if (event.key === "Escape") {
              event.preventDefault();
              onRenameCancel();
            }
          }}
        />
      ) : (
        <RowBody row={row} projectStatus={projectStatus} />
      )}

      {row.kind === "conversation" ? (
        <ConversationRowActions
          conversation={row.conversation}
          disabled={!chatReady}
          onDelete={onDeleteConversation}
          onPin={onPinConversation}
        />
      ) : null}

      {/* What a project row can do, in the order a person needs it. The new
          chat button stays on screen and the remove button does not: a row
          should advertise the thing you came to do and hold back the thing you
          cannot undo. Starting a chat in a project had no affordance at all --
          it lived in the context menu, which is a place you look only once you
          already believe the action exists. */}
      {row.kind === "project" ? (
        <span className="sidebar-row__actions sidebar-row__actions--container">
          <IconButton
            aria-label={`New chat in ${row.project.display_name}`}
            disabled={!chatReady}
            size="sm"
            onClick={(event) => {
              // The row underneath opens and closes the project; this fills it.
              event.stopPropagation();
              onStartProjectChat(row.project.id);
            }}
          >
            <MessageSquarePlus aria-hidden size={14} />
          </IconButton>
          <IconButton
            aria-label={`Remove ${row.project.display_name}`}
            className="sidebar-row__action--quiet"
            size="sm"
            variant="danger"
            onClick={(event) => {
              // The row underneath opens the project; this takes it away.
              event.stopPropagation();
              onRemoveProject(row.project);
            }}
          >
            <Trash2 aria-hidden size={14} />
          </IconButton>
        </span>
      ) : null}
    </div>
  );
}

/**
 * A row's label, and only what that particular row needs beside it.
 *
 * No per-row type icon. Every row in this tree carried one, and a dozen of
 * them stacked in a 17rem column read as texture rather than as information --
 * the icons were identical down each run, so they distinguished nothing and
 * spent the width a chat's name needed. What a row is now comes from where it
 * sits: its indent, its disclosure chevron, and the weight a container carries.
 * The icons that survive are the ones that are NOT repeated -- the section
 * titles, and the sensitivity mark on the few files that earn it.
 */
function RowBody({
  row,
  projectStatus,
}: {
  row: NavRow;
  projectStatus?: Readonly<Record<string, ProjectIndexStatus>>;
}) {
  switch (row.kind) {
    case "project":
      if (projectStatus?.[row.project.id]) {
        return (
          <div className="min-w-0 flex-1">
            <TruncatedText as="div">{row.project.display_name}</TruncatedText>
            <ProjectStatus
              className="mt-1 break-words font-normal"
              interactive={false}
              status={projectStatus[row.project.id]}
            />
          </div>
        );
      }
      return <TruncatedText>{row.project.display_name}</TruncatedText>;
    case "conversation":
      return <TruncatedText className="sidebar-row__title">{conversationTitle(row.conversation)}</TruncatedText>;
    case "file":
      return (
        <>
          <Tooltip label={row.file.path}>
            <span>{fileName(row.file.path)}</span>
          </Tooltip>
          {row.file.sensitive ? (
            <span className="ml-auto flex shrink-0 items-center text-warn">
              <ShieldAlert aria-hidden size={13} strokeWidth={1.9} />
              <span className="sr-only">Classified as sensitive — not read</span>
            </span>
          ) : null}
        </>
      );
    case "browse":
      // An action into the project's folder, not a conversation: drawn with a
      // folder icon and in the quieter caption voice, so it does not read as
      // one more chat in the list above it.
      return (
        <>
          <FolderOpen aria-hidden className="sidebar-row__browse-icon" size={13} strokeWidth={1.9} />
          <span>Browse files</span>
        </>
      );
    case "note":
      return <span>{row.text}</span>;
  }
}
