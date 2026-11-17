import { useDeferredValue, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  ChartNoAxesColumn,
  Database,
  FileText,
  FolderTree,
  MessageSquare,
  MessageSquarePlus,
  Search,
  Settings,
  Sparkles,
} from "lucide-react";
import type {
  ChatConversationSummary,
  ConversationId,
  McpServerSummary,
  ProjectView,
  ReportView,
  SkillView,
} from "../lib/generated/contracts";
import { scoreSearch } from "../lib/searchText";
import { groupConversations } from "../features/chat/conversationOrder";
import { DialogShell } from "../ui/DialogShell";
import { Shortcut } from "../ui/Kbd";
import { Spinner } from "../ui/Spinner";
import {
  SETTINGS_DESTINATIONS,
  settingsDestinationReveal,
  settingsItemElementId,
  type SettingsReveal,
} from "./settingsCatalog";
import { PALETTE_GROUPS, rankWorkspaceSearch, type PaletteGroup, type WorkspaceSearchRecord } from "./workspaceSearch";

export interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
  onNewChat: () => void;
  /** Every conversation the shell holds, used to label and order chat matches. */
  conversations: ChatConversationSummary[];
  projects?: readonly ProjectView[];
  reports?: readonly ReportView[];
  mcpServers?: readonly McpServerSummary[];
  skills?: readonly SkillView[];
  /** The open transcript, searched in memory so a phrase in this chat is instant. */
  openConversationId?: ConversationId | null;
  openMessageTexts?: readonly string[];
  searchConversationMessages: (query: string) => Promise<ConversationId[]>;
  onOpenConversation: (conversationId: ConversationId, query: string) => void;
  onOpenProject?: (projectId: string, query: string) => void;
  onOpenReport?: (reportId: string, query: string) => void;
  onOpenSettingsSection?: (reveal: Omit<SettingsReveal, "token">) => void;
  /**
   * The remaining destinations are optional so the palette can only ever offer a
   * surface the shell actually wired: an unlisted command is one this build has
   * no way to reach, which is exactly what should not appear here.
   */
  onGoToChat?: () => void;
  onBrowseFiles?: () => void;
  onOpenDatasets?: () => void;
  onOpenReports?: () => void;
  onOpenSettings?: () => void;
}

interface PaletteEntry {
  id: string;
  label: string;
  hint?: string;
  icon: ReactNode;
  group: PaletteGroup;
  conversationId?: ConversationId;
  run: (query: string) => void;
}

export function CommandPalette({
  open,
  onClose,
  onNewChat,
  conversations,
  projects = [],
  reports = [],
  mcpServers = [],
  skills = [],
  openConversationId = null,
  openMessageTexts = [],
  searchConversationMessages,
  onOpenConversation,
  onOpenProject,
  onOpenReport,
  onOpenSettingsSection,
  onGoToChat,
  onBrowseFiles,
  onOpenDatasets,
  onOpenReports,
  onOpenSettings,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const [messageMatches, setMessageMatches] = useState<Set<ConversationId>>(new Set());
  const [messageSearchState, setMessageSearchState] = useState<"idle" | "searching" | "ready" | "error">("idle");
  const inputRef = useRef<HTMLInputElement>(null);
  const listboxRef = useRef<HTMLDivElement>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);
  const listboxId = useId();

  const commands = useMemo<PaletteEntry[]>(() => {
    const available: (PaletteEntry | null)[] = [
      {
        id: "new-chat",
        label: "New chat",
        icon: <MessageSquarePlus aria-hidden size={16} strokeWidth={1.8} />,
        group: "Commands",
        run: onNewChat,
      },
      onGoToChat
        ? {
            id: "go-to-chat",
            label: "Go to chat",
            icon: <MessageSquare aria-hidden size={16} strokeWidth={1.8} />,
            group: "Commands",
            run: onGoToChat,
          }
        : null,
      onBrowseFiles
        ? {
            id: "browse-files",
            label: "Browse project artifacts",
            icon: <FolderTree aria-hidden size={16} strokeWidth={1.8} />,
            group: "Commands",
            run: onBrowseFiles,
          }
        : null,
      onOpenDatasets
        ? {
            id: "open-datasets",
            label: "Datasets",
            icon: <Database aria-hidden size={16} strokeWidth={1.8} />,
            group: "Commands",
            run: onOpenDatasets,
          }
        : null,
      onOpenReports
        ? {
            id: "open-reports",
            label: "Reports",
            icon: <ChartNoAxesColumn aria-hidden size={16} strokeWidth={1.8} />,
            group: "Commands",
            run: onOpenReports,
          }
        : null,
      onOpenSettings
        ? {
            id: "open-settings",
            label: "Settings",
            icon: <Settings aria-hidden size={16} strokeWidth={1.8} />,
            group: "Commands",
            run: onOpenSettings,
          }
        : null,
    ];
    return available.filter((command): command is PaletteEntry => command !== null);
  }, [onBrowseFiles, onGoToChat, onNewChat, onOpenDatasets, onOpenReports, onOpenSettings]);

  const catalog = useMemo(() => {
    const records: WorkspaceSearchRecord[] = [];
    const entries = new Map<string, PaletteEntry>();

    groupConversations(conversations).ordered.forEach((conversation, order) => {
      const id = `conversation-${conversation.conversation_id}`;
      records.push({
        id,
        group: "Chats",
        label: conversation.title || "Untitled conversation",
        order,
        pinned: Boolean(conversation.pinned_at),
      });
      entries.set(id, {
        id,
        label: conversation.title || "Untitled conversation",
        icon: <MessageSquare aria-hidden size={16} strokeWidth={1.8} />,
        group: "Chats",
        conversationId: conversation.conversation_id,
        run: (query) => onOpenConversation(conversation.conversation_id, query),
      });
    });

    projects.forEach((project, order) => {
      const id = `project-${project.id}`;
      records.push({
        id,
        group: "Projects",
        label: project.display_name,
        keywords: project.root_path,
        order,
      });
      entries.set(id, {
        id,
        label: project.display_name,
        hint: project.root_path,
        icon: <FolderTree aria-hidden size={16} strokeWidth={1.8} />,
        group: "Projects",
        run: (query) => onOpenProject?.(project.id, query),
      });
    });

    reports.forEach((report, order) => {
      const id = `report-${report.id}`;
      records.push({
        id,
        group: "Reports",
        label: report.title,
        keywords: report.description ?? undefined,
        order,
        pinned: report.pinned,
      });
      entries.set(id, {
        id,
        label: report.title,
        hint: report.description ?? undefined,
        icon: <FileText aria-hidden size={16} strokeWidth={1.8} />,
        group: "Reports",
        run: (query) => onOpenReport?.(report.id, query),
      });
    });

    const openSettings = (reveal: Omit<SettingsReveal, "token">): void => {
      if (onOpenSettingsSection) {
        onOpenSettingsSection(reveal);
        return;
      }
      onOpenSettings?.();
    };

    if (onOpenSettings || onOpenSettingsSection) {
      SETTINGS_DESTINATIONS.forEach((destination, order) => {
        const id = `settings-${destination.rowId}`;
        records.push({
          id,
          group: "Settings",
          label: destination.label,
          keywords: destination.keywords,
          order,
        });
        entries.set(id, {
          id,
          label: destination.label,
          icon: <Settings aria-hidden size={16} strokeWidth={1.8} />,
          group: "Settings",
          run: (query) => openSettings(settingsDestinationReveal(destination.rowId, query)),
        });
      });

      mcpServers.forEach((server, order) => {
        const id = `mcp-${server.id}`;
        records.push({
          id,
          group: "Settings",
          label: server.name,
          keywords: `mcp server ${server.id}`,
          order: SETTINGS_DESTINATIONS.length + order,
        });
        entries.set(id, {
          id,
          label: server.name,
          hint: "MCP server",
          icon: <Settings aria-hidden size={16} strokeWidth={1.8} />,
          group: "Settings",
          run: (query) =>
            openSettings({
              section: "models-connections",
              targetId: settingsItemElementId("mcp", server.id),
              query,
            }),
        });
      });

      skills.forEach((skill, order) => {
        const id = `skill-${skill.name}`;
        records.push({
          id,
          group: "Settings",
          label: skill.name,
          keywords: `skill ${skill.description}`,
          order: SETTINGS_DESTINATIONS.length + mcpServers.length + order,
        });
        entries.set(id, {
          id,
          label: skill.name,
          hint: skill.description,
          icon: <Sparkles aria-hidden size={16} strokeWidth={1.8} />,
          group: "Settings",
          run: (query) =>
            openSettings({
              section: "agent-capabilities",
              targetId: settingsItemElementId("skill", skill.name),
              query,
            }),
        });
      });
    }

    commands.forEach((command, order) => {
      records.push({
        id: command.id,
        group: "Commands",
        label: command.label,
        order,
      });
      entries.set(command.id, command);
    });

    return { records, entries };
  }, [
    commands,
    conversations,
    mcpServers,
    onOpenConversation,
    onOpenProject,
    onOpenReport,
    onOpenSettings,
    onOpenSettingsSection,
    projects,
    reports,
    skills,
  ]);

  // The transcript scan is the expensive part, so it runs at lower priority than
  // the keystroke, and only while the palette is open.
  const deferredQuery = useDeferredValue(query);
  const openBodyMatch = useMemo(() => {
    const trimmed = deferredQuery.trim();
    if (!open || !openConversationId || trimmed.length === 0) return false;
    return openMessageTexts.some((text) => scoreSearch(text, trimmed) !== null);
  }, [deferredQuery, open, openConversationId, openMessageTexts]);

  useEffect(() => {
    const trimmed = query.trim();
    if (!open || trimmed.length === 0) {
      setMessageMatches(new Set());
      setMessageSearchState("idle");
      return;
    }

    let cancelled = false;
    setMessageSearchState("searching");
    const timer = window.setTimeout(() => {
      void searchConversationMessages(trimmed)
        .then((conversationIds) => {
          if (cancelled) return;
          setMessageMatches(new Set(conversationIds));
          setMessageSearchState("ready");
        })
        .catch(() => {
          if (cancelled) return;
          setMessageMatches(new Set());
          setMessageSearchState("error");
        });
    }, 120);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [open, query, searchConversationMessages]);

  const matches = useMemo(() => {
    const extra = new Set<string>();
    for (const conversationId of messageMatches) {
      extra.add(`conversation-${conversationId}`);
    }
    if (openBodyMatch && openConversationId) {
      extra.add(`conversation-${openConversationId}`);
    }

    return rankWorkspaceSearch(catalog.records, query, extra).flatMap((hit) => {
      const entry = catalog.entries.get(hit.id);
      if (!entry) return [];
      const fromBody =
        entry.conversationId !== undefined &&
        (messageMatches.has(entry.conversationId) || (openBodyMatch && entry.conversationId === openConversationId));
      const titleMissed = scoreSearch(entry.label, query) === null && query.trim().length > 0;
      return [
        {
          ...entry,
          hint: fromBody && titleMissed ? (entry.hint ?? "Matched in messages") : entry.hint,
        },
      ];
    });
  }, [catalog, messageMatches, openBodyMatch, openConversationId, query]);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setActiveIndex(0);
    // Ahead of the shell's own focus move, which then sees the panel already
    // holds focus and leaves it here rather than on the first control.
    inputRef.current?.focus();
  }, [open]);

  /**
   * Keeps the active option inside the result set whenever it changes for a
   * reason other than a keystroke -- the debounced message search landing is
   * the one that matters. Typing already resets to 0 on every change, but a
   * background result arriving later can shrink `matches` out from under
   * whatever index was active, and unlike a keystroke, that arrival fires no
   * handler that would otherwise re-clamp it. Left alone, `matches[activeIndex]`
   * goes `undefined`: no option carries `aria-selected`, `aria-activedescendant`
   * points at nothing, and Enter runs nothing, all while a visible result sits
   * right there.
   */
  useEffect(() => {
    setActiveIndex((current) => (matches.length === 0 ? 0 : Math.min(current, matches.length - 1)));
  }, [matches.length]);

  /**
   * Keeps the active option on screen. Arrow keys move `aria-selected`, but the
   * list only scrolled for the wheel, so the selection walked off the bottom of
   * the panel after a few rows and stayed hidden. The first option scrolls the
   * list to its very top, so its group heading shows with it.
   */
  useLayoutEffect(() => {
    if (!open) return;
    const listbox = listboxRef.current;
    const active = matches[activeIndex];
    if (!listbox || !active) return;
    if (activeIndex === 0) {
      listbox.scrollTop = 0;
      return;
    }
    document.getElementById(`${listboxId}-${active.id}`)?.scrollIntoView?.({ block: "nearest" });
  }, [activeIndex, listboxId, matches, open]);

  if (!open) return null;

  const runEntry = (entry: PaletteEntry | undefined): void => {
    if (!entry) return;
    entry.run(query.trim());
    onClose();
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>): void => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (matches.length === 0) return;
      const direction = event.key === "ArrowDown" ? 1 : -1;
      setActiveIndex((current) => (current + direction + matches.length) % matches.length);
      return;
    }

    if (event.key === "Enter") {
      event.preventDefault();
      runEntry(matches[activeIndex]);
    }
  };

  const activeEntry = matches[activeIndex];

  return (
    <DialogShell
      className="command-palette__dialog xf-material xf-overlay-pop"
      label="Search"
      onDismiss={onClose}
      open={open}
      placement="top"
    >
      <div className="command-palette__search">
        <Search aria-hidden size={16} strokeWidth={1.8} />
        <input
          ref={inputRef}
          type="search"
          role="combobox"
          aria-label="Search workspace"
          aria-autocomplete="list"
          aria-expanded="true"
          aria-controls={listboxId}
          aria-activedescendant={activeEntry ? `${listboxId}-${activeEntry.id}` : undefined}
          autoComplete="off"
          placeholder="Search chats, projects, reports, settings…"
          value={query}
          onChange={(event) => {
            setQuery(event.currentTarget.value);
            setActiveIndex(0);
          }}
          onKeyDown={handleKeyDown}
        />
      </div>

      <div id={listboxId} ref={listboxRef} role="listbox" aria-label="Results">
        {PALETTE_GROUPS.map((group) => {
          const entries = matches.filter((entry) => entry.group === group);
          if (entries.length === 0) return null;
          return (
            <div key={group} role="group" aria-label={group}>
              <p className="command-palette__group" aria-hidden>
                {group}
              </p>
              {entries.map((entry) => {
                const index = matches.indexOf(entry);
                return (
                  <button
                    key={entry.id}
                    id={`${listboxId}-${entry.id}`}
                    type="button"
                    role="option"
                    aria-selected={index === activeIndex}
                    onPointerMove={(event) => {
                      // Only a pointer that actually moved takes the selection.
                      // Scrolling the list under a resting pointer fires moves
                      // at the same spot, and those would snap the selection
                      // back under the pointer mid-way through a key press.
                      const last = lastPointerRef.current;
                      lastPointerRef.current = { x: event.clientX, y: event.clientY };
                      if (last && last.x === event.clientX && last.y === event.clientY) return;
                      setActiveIndex(index);
                    }}
                    onClick={() => runEntry(entry)}
                  >
                    {entry.icon}
                    <span className="command-palette__option-copy">
                      <span>{entry.label}</span>
                      {entry.hint ? <span className="command-palette__hint">{entry.hint}</span> : null}
                    </span>
                  </button>
                );
              })}
            </div>
          );
        })}

        {matches.length === 0 ? (
          messageSearchState === "searching" ? (
            // The busy pattern the rest of the shell uses for a named wait --
            // App.tsx's own startup screen pairs the same decorative spinner
            // with adjacent visible text, inside one live region, rather than
            // stating "searching" as text alone with nothing to look at.
            <p role="status" aria-live="polite" className="command-palette__status">
              <Spinner decorative size={14} />
              Searching chats…
            </p>
          ) : (
            <p role="status" className="command-palette__status">
              {messageSearchState === "error" ? "Chat message search is unavailable" : "No matches"}
            </p>
          )
        ) : null}
      </div>

      <footer className="command-palette__footer" aria-hidden>
        <span className="inline-flex items-center gap-1">
          <Shortcut keys={["Up"]} />
          <Shortcut keys={["Down"]} /> navigate
        </span>
        <span className="inline-flex items-center gap-1">
          <Shortcut keys={["Enter"]} /> run · <Shortcut keys={["Escape"]} /> close
        </span>
      </footer>
    </DialogShell>
  );
}
