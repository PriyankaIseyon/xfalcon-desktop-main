import { SettingsUpdateBadge } from "../features/settings/SettingsUpdateBadge";
import { UpdateNotice } from "../features/settings/UpdateNotice";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  useTransition,
  Suspense,
} from "react";
import {
  FileDown,
  MessageSquarePlus,
  PanelLeftClose,
  PanelLeftOpen,
  PanelRightClose,
  PanelRightOpen,
  Search,
} from "lucide-react";
import type { AppBootstrap, ChatConversationSummary, ProjectView, SkillView } from "../lib/generated/contracts";
import { CanvasSlot } from "../features/canvas/CanvasSlot";
import { clearCanvasArtifacts, toggleCanvasOpen, useCanvasStore } from "../features/canvas/canvasStore";
import { WorkspaceSkeleton } from "../features/chat/WorkspaceSkeleton";
import { ChatPane } from "../features/chat/ChatPane";
import { ConversationList } from "../features/chat/ConversationList";
import { ConversationTitle } from "../features/chat/ConversationTitle";
import { useChat } from "../features/chat/useChat";
import { startChatEventBridge, useChatSelector } from "../features/chat/chatStore";
import { isBoundRunPending } from "../features/chat/runState";
import { useWorkspaceConnection } from "../features/workspace/useWorkspaceConnection";
import { ProjectNavList } from "../features/projects/ProjectNavList";
import { chatFiles } from "../features/projects/chatFiles";
import { useDatasetDrops } from "../features/datasets/useDatasetDrops";
import {
  type ProjectReportDestination,
  ReportDestinationContext,
  type ReportProjectChoices,
  ReportProjectsContext,
} from "../features/reports/reportDestination";
import { isListed } from "../features/reports/reportPresentation";
import {
  applyReportList,
  beginReportListRequest,
  cancelReportListRequest,
  getPendingReportId,
  getReportsSnapshot,
  isCurrentReportListRequest,
  requestOpenReport,
  subscribePendingReport,
  subscribeReports,
} from "../features/reports/reportsStore";
import { McpReauthPrompt } from "../features/mcp/McpReauthPrompt";
import {
  authLogout,
  chatConversationRename,
  chatConversationSearch,
  projectActivate,
  projectDeactivate,
  projectList,
  projectRevoke,
  reportList,
  skillList,
} from "../lib/native";
import { toast } from "../ui/toastStore";
import { idleSignOutNotice, useIdleSignOut } from "./useIdleSignOut";
import { setAuthSession } from "../features/auth/authStore";
import { setSignedOutNotice } from "../features/auth/signedOutNotice";
import { useAuthSession } from "../features/auth/useAuth";
import { AlertDialog } from "../ui/AlertDialog";
import { IconButton } from "../ui/IconButton";
import { Shortcut } from "../ui/Kbd";
import { Tooltip } from "../ui/Tooltip";
import { TruncatedText } from "../ui/TruncatedText";
import { Logo } from "../ui/Logo";
import { AccountMenu } from "./AccountMenu";
import { DestinationNav, SidebarRail } from "./WorkspaceNav";
import { SidebarResizer } from "./SidebarResizer";
import { SETTINGS_DESTINATION, type WorkspaceView } from "./destinations";
import { CommandPalette } from "./CommandPalette";
import { ShortcutHelpDialog } from "./ShortcutHelpDialog";
import {
  CanvasPanel,
  ChatExportDialog,
  DatasetsView,
  ProjectExplorerView,
  ReportsView,
  SettingsView,
  preloadWorkspaceViews,
} from "./lazyViews";
import { useAppLayout } from "./useAppLayout";
import { useOnlineStatus } from "./useOnlineStatus";
import { OfflineBanner } from "../ui/OfflineBanner";
import { settingsDestinationReveal, settingsSectionReveal, type SettingsReveal } from "./settingsCatalog";
import { cn } from "../lib/cn";

const SettingsIcon = SETTINGS_DESTINATION.icon;

export interface AppShellProps {
  bootstrap: AppBootstrap;
}

/**
 * Every trusted project for the current account, revalidated fresh on
 * mount/refresh (see `ProjectsService::list`'s own doc comment) -- kept here,
 * not inside `ProjectNavList`, so activation and creation can both trigger
 * the same single refetch and stay consistent with each other.
 */
function useProjects() {
  const [projects, setProjects] = useState<ProjectView[]>([]);
  const [error, setError] = useState<string | null>(null);
  const refreshRequestRef = useRef(0);

  const refresh = useCallback(async () => {
    const request = ++refreshRequestRef.current;
    try {
      // Cleanup can publish deletions even when a later operation fails.
      await startChatEventBridge();
      if (request !== refreshRequestRef.current) return;
      const result = await projectList();
      if (request !== refreshRequestRef.current) return;
      setProjects(result.projects);
      setError(null);
    } catch (cause) {
      if (request === refreshRequestRef.current) {
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    }
  }, []);

  useEffect(() => {
    void refresh();
    const revalidate = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    window.addEventListener("focus", revalidate);
    document.addEventListener("visibilitychange", revalidate);
    return () => {
      refreshRequestRef.current += 1;
      window.removeEventListener("focus", revalidate);
      document.removeEventListener("visibilitychange", revalidate);
    };
  }, [refresh]);

  const activate = useCallback(
    async (projectId: string) => {
      try {
        await projectActivate(projectId);
        setError(null);
        await refresh();
        return true;
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : String(cause));
      }
      // Re-sync with the backend's own truth either way -- a failed
      // activation may still have left the previous active project intact,
      // and this is the only place that knows what it actually is now.
      await refresh();
      return false;
    },
    [refresh],
  );

  // A successful revoke is already authoritative. Remove that project from
  // the local view before the follow-up list call, so a list refresh failure
  // cannot leave a revoked project offering file actions in the sidebar.
  const forget = useCallback((projectId: string) => {
    setProjects((current) => current.filter((project) => project.id !== projectId));
  }, []);

  return { projects, error, refresh, activate, forget };
}

/** Skeleton cap (spec: workspace-arrival): force-reveal after 5 s so a hung
 * bootstrap can never hold the shell hostage. */
const WORKSPACE_ARRIVAL_CAP_MS = 5000;

/** Stable identity for a project with no conversations, so its list prop does
 * not change on every render and re-run the list's own effects. */
const EMPTY_CONVERSATIONS: ChatConversationSummary[] = [];

/** Each destination's accessible region name. Every value must be unique --
 * shell tests select the main region by accessible name. */
const MAIN_LABEL: Record<WorkspaceView, string> = {
  chat: "Chat workspace",
  projects: "Project files",
  datasets: "Datasets",
  reports: "Reports",
  settings: "Settings",
};

const VIEW_SUBTITLE: Record<WorkspaceView, string | null> = {
  chat: null,
  projects: "Files",
  datasets: "Workspace data",
  reports: "Saved dashboards",
  settings: "Models and connections",
};

/** What the titlebar can name a destination after. */
type TitlebarContext = {
  conversationTitle: string;
  projectName: string | null;
};

/**
 * Each destination's titlebar name, as a map total over `WorkspaceView`.
 *
 * This was a ternary chain whose final else-branch was the files case, so a
 * new destination compiled clean and then titled itself with the active
 * project's name. A `Record` keyed on the union has no else-branch to fall
 * into: an unnamed destination is a compile error.
 */
const TITLEBAR_NAME: Record<WorkspaceView, (context: TitlebarContext) => string> = {
  chat: (context) => context.conversationTitle,
  projects: (context) => context.projectName ?? "Project files",
  datasets: () => "Datasets",
  reports: () => "Reports",
  settings: () => "Settings",
};

/** The one shortcut this shell prints in two places. Naming it here is what
 * kept the sidebar and the rail from drifting to `Ctrl K` and `Ctrl+K`. */
const COMMAND_PALETTE_KEYS = ["Mod", "K"] as const;

export function AppShell({ bootstrap }: AppShellProps) {
  const session = useAuthSession();
  const { isNarrow, sidebarOpen, setSidebarOpen, toggleSidebar } = useAppLayout();
  const online = useOnlineStatus();
  /**
   * `OfflineBanner` unmounts the moment `online` flips back to true, and an
   * unmount is not a change a `role="status"` region announces -- only an
   * addition or update inside one is. Going offline gets an announcement for
   * free from the banner appearing; coming back got none at all, so a screen
   * reader user (or anyone who was not looking at that exact spot when the
   * banner disappeared) had no way to know connectivity was back short of
   * trying an action and seeing whether it now worked. The same shape MCP
   * reconnection already uses (`toast.success` on the transition back),
   * skipped on first mount so a session that starts online does not open with
   * an unearned "back online."
   */
  const wasOffline = useRef(false);
  useEffect(() => {
    if (!online) {
      wasOffline.current = true;
      return;
    }
    if (wasOffline.current) {
      wasOffline.current = false;
      toast.success("You're back online.");
    }
  }, [online]);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [mcpRevision, setMcpRevision] = useState(0);
  const [settingsFocus, setSettingsFocus] = useState<SettingsReveal | null>(null);
  // A "+" on a destination asks that destination to start something new once
  // it is showing; the destination clears it when it has acted, so returning
  // to the page later never repeats it.
  const [createRequest, setCreateRequest] = useState<"projects" | "datasets" | null>(null);
  const [jumpQuery, setJumpQuery] = useState("");
  const [paletteSkills, setPaletteSkills] = useState<SkillView[]>([]);
  const reportsSnap = useSyncExternalStore(subscribeReports, getReportsSnapshot, getReportsSnapshot);
  // A report the reader hid is not one they asked to be offered.
  const paletteReports = useMemo(() => reportsSnap.reports.filter(isListed), [reportsSnap.reports]);
  const [exportOpen, setExportOpen] = useState(false);
  const closeExport = useCallback(() => setExportOpen(false), []);
  const canvasState = useCanvasStore();
  const canvasOpen = canvasState.isOpen && canvasState.tabs.length > 0;
  // Window drag is macOS only: its title bar is an overlay, and the drag permissions are granted there alone.
  const windowDrag = bootstrap.platform === "macos";
  const identity = session.status === "authenticated" ? session.identity : null;
  const inTauriShell = Boolean((globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);
  const workspace = useWorkspaceConnection();
  const chatCapability = workspace.status?.chat_ready ?? null;
  const chat = useChat(chatCapability, identity != null && inTauriShell);
  // The chat bootstrap is itself an authenticated data-plane request. Keep an
  // explicit `false` authoritative, but do not label chat disconnected merely
  // because the parallel auxiliary status summary failed.
  const chatReady = workspace.status?.chat_ready ?? chat.workspaceLoaded;
  const conversationId = chat.conversationId;
  const [pendingChatDelete, setPendingChatDelete] = useState<ChatConversationSummary | null>(null);
  const [chatDeleteBusy, setChatDeleteBusy] = useState(false);
  const [chatDeleteError, setChatDeleteError] = useState<string | null>(null);
  const chatDeleteInFlight = useRef(false);
  const requestChatDelete = (targetId: string) => {
    const target = chat.conversations.find((conversation) => conversation.conversation_id === targetId);
    if (!target || chatDeleteInFlight.current) return;
    setChatDeleteError(null);
    setPendingChatDelete(target);
  };
  const confirmChatDelete = async () => {
    if (!pendingChatDelete || chatDeleteInFlight.current) return;
    chatDeleteInFlight.current = true;
    setChatDeleteBusy(true);
    setChatDeleteError(null);
    try {
      if (await chat.deleteConversation(pendingChatDelete.conversation_id)) {
        setPendingChatDelete(null);
      } else {
        setChatDeleteError("The chat could not be deleted. Try again.");
      }
    } finally {
      chatDeleteInFlight.current = false;
      setChatDeleteBusy(false);
    }
  };
  /**
   * A canvas artifact belongs to the conversation that produced it.
   *
   * Cleared on every change of conversation — including the change to `null` a
   * sign-out produces — because a table left open across a switch would show one
   * chat's data on another chat's screen, and after a sign-out the previous
   * account's. The transcript itself is replaced by `useChat`; nothing was
   * replacing the panel beside it.
   */
  useEffect(() => {
    clearCanvasArtifacts();
  }, [conversationId]);
  const projects = useProjects();
  const activeProject = projects.projects.find((project) => project.active);
  const activeProjectId = activeProject?.id ?? null;
  // Every project's own conversations, grouped once rather than re-filtered
  // per project row -- the sidebar renders all of them, not just the active
  // project's.
  const conversationsByProject = useMemo(() => {
    const grouped = new Map<string, ChatConversationSummary[]>();
    for (const conversation of chat.conversations) {
      if (conversation.project_id == null) continue;
      const existing = grouped.get(conversation.project_id);
      if (existing) existing.push(conversation);
      else grouped.set(conversation.project_id, [conversation]);
    }
    return grouped;
  }, [chat.conversations]);
  const conversationsFor = useCallback(
    (projectId: string) => conversationsByProject.get(projectId) ?? EMPTY_CONVERSATIONS,
    [conversationsByProject],
  );
  /** Chats lists the loose conversations; a project's own are rows of its tree. */
  const recentConversations = useMemo(
    () => chat.conversations.filter((conversation) => conversation.project_id == null),
    [chat.conversations],
  );
  const conversationProject = useMemo(() => {
    const projectId = chat.conversations.find(
      (conversation) => conversation.conversation_id === chat.conversationId,
    )?.project_id;
    return projectId == null ? null : (projects.projects.find((project) => project.id === projectId) ?? null);
  }, [chat.conversationId, chat.conversations, projects.projects]);
  /**
   * The files the open conversation used, for its branch of the sidebar tree.
   * Derived from the projected messages already in hand -- a conversation
   * summary carries no tool steps, so this is the one chat whose files can be
   * named without a round trip per chat.
   */
  const activeConversationFiles = useMemo(() => chatFiles(chat.messages), [chat.messages]);
  const projectFiles = useMemo(() => {
    return activeConversationFiles.files.map((f) => f.path);
  }, [activeConversationFiles]);

  const mcpServers = useMemo(
    () => workspace.status?.mcp_servers?.filter((s) => s.enabled).map((s) => s.name) ?? [],
    [workspace.status?.mcp_servers],
  );

  const activeChatProjectName =
    chat.conversationId == null ? (activeProject?.display_name ?? null) : (conversationProject?.display_name ?? null);
  const activeChatProjectUnavailable =
    (chat.conversationId == null ? activeProject : conversationProject)?.status === "unavailable";
  const projectStatus = useMemo(
    () =>
      Object.fromEntries(
        projects.projects.flatMap((project) =>
          project.status === "unavailable"
            ? [[project.id, { state: "error" as const, detail: "Project folder unavailable" }]]
            : [],
        ),
      ),
    [projects.projects],
  );
  const [view, setView] = useState<WorkspaceView>("chat");
  const [selectedDestination, setSelectedDestination] = useState<WorkspaceView>("chat");
  const [, startViewTransition] = useTransition();
  const [shortcutHelpOpen, setShortcutHelpOpen] = useState(false);
  /**
   * Whether the sidebar shows the things you do with chats.
   *
   * A destination owns the column below the group, the way the web app's does.
   * Chats owns the recents list; Projects owns the tree; Datasets and Reports
   * bring no list, so on those the column is theirs to leave empty. Nothing is
   * drawn on a destination it does not belong to -- a conversation reading as
   * open while Reports is on screen is the state this rule exists to prevent.
   */
  const chatSurface = view === "chat";
  const projectSurface = view === "projects";

  /**
   * What the Projects destination is showing beside its tree.
   *
   * A project holds files AND the conversations you had about it, and both are
   * reached from the same tree -- so pressing either must not cost the reader
   * the column they are reading. The destination hosts whichever one they
   * picked, and stays the current destination while it does. Without this a
   * project's chat could only be opened by leaving Projects for Chats, which is
   * a trapdoor, not navigation.
   */
  const [projectPane, setProjectPane] = useState<"files" | "chat">("files");
  /** Whether the transcript is what the main region is showing, on either
   * destination that can show one. */
  const chatPaneVisible = chatSurface || (projectSurface && projectPane === "chat");
  const titlebarSubtitle = projectSurface
    ? projectPane === "chat"
      ? (activeProject?.display_name ?? "Project")
      : "Files"
    : VIEW_SUBTITLE[view];

  /**
   * Closed means two different things, and conflating them is why collapsing
   * the sidebar used to cost the whole of the app's navigation.
   *
   * With room for it, the sidebar collapses to a rail: still on screen, still
   * interactive, every destination one press away. On a narrow window there is
   * no room for a rail beside the workspace, so the panel is a drawer that
   * leaves entirely -- and while it is gone it must be inert, or the tab order
   * still runs through a sidebar nobody can see.
   */
  const railed = !sidebarOpen && !isNarrow;
  const sidebarHidden = !sidebarOpen && isNarrow;
  /**
   * The file the project file view has open, carried with the project it
   * belongs to so switching projects cannot leave the previous one's path
   * selected. Owned here because two surfaces open files now -- the tree in
   * that view, and a chat's own file rows in the sidebar.
   */
  const [openFile, setOpenFile] = useState<{ projectId: string; path: string } | null>(null);
  const [mcpSelectionChanging, setMcpSelectionChanging] = useState(false);
  const openFilePath = openFile != null && openFile.projectId === activeProject?.id ? openFile.path : null;

  const showView = useCallback(
    (next: WorkspaceView) => {
      // A "+" request belongs to the destination it was made for. If the person
      // leaves before that destination mounts, the request is dropped, or it
      // would open the picker or sheet on some later visit.
      setCreateRequest((request) => (request === next ? request : null));
      // Move the sidebar selection in the urgent render. The heavier workspace
      // surface follows in a transition, so it cannot hold the navigation pill
      // on the previous destination while its first render is prepared.
      setSelectedDestination(next);
      startViewTransition(() => setView(next));
      if (isNarrow) setSidebarOpen(false);
    },
    [isNarrow, setSidebarOpen, startViewTransition],
  );

  /** The file pane is a lazy view, so it changes in a transition: an urgent
   * change would unmount the chat before the explorer chunk loads. */
  const showProjectFiles = useCallback(() => {
    startViewTransition(() => setProjectPane("files"));
  }, [startViewTransition]);

  /**
   * The one way the sidebar navigates, so the group and the rail cannot end up
   * doing different things for the same destination. Settings arrives with no
   * section revealed unless something asked for one; reaching it from the
   * sidebar is never that ask.
   */
  const goToDestination = useCallback(
    (next: WorkspaceView) => {
      if (next === "settings") setSettingsFocus(null);
      // A destination has a home, and pressing it in the group goes there.
      // Projects can also be showing one of a project's chats, chosen from its
      // tree -- but that is somewhere you navigated TO, not what the
      // destination is, so pressing Projects comes back to the files.
      if (next === "projects") showProjectFiles();
      showView(next);
    },
    [showProjectFiles, showView],
  );

  /**
   * A dataset dropped on the window brings the person to the destination.
   *
   * Owned here, not by the destination, because the backend stages the file
   * whatever is on screen: a listener that lived in a view mounted only while
   * that view was showing would leave the staged file with nothing to claim it.
   * The same shape as `pendingReportId` above — the backend records an intent,
   * the shell routes to it, the destination consumes it.
   */
  const openDroppedDataset = useCallback(() => showView("datasets"), [showView]);
  useDatasetDrops(view === "datasets", openDroppedDataset);

  const handleSlashCommand = useCallback(
    (cmd: string) => {
      switch (cmd) {
        case "clear":
          void chat.startNewConversation();
          break;
        case "help":
          setShortcutHelpOpen(true);
          break;
        case "project":
          showView("projects");
          break;
        case "mcp":
          setSettingsFocus({ ...settingsDestinationReveal("mcp", ""), token: Date.now() });
          showView("settings");
          break;
        case "skill":
          setSettingsFocus({ ...settingsDestinationReveal("skills", ""), token: Date.now() });
          showView("settings");
          break;
        case "model":
          setSettingsFocus({ ...settingsDestinationReveal("models", ""), token: Date.now() });
          showView("settings");
          break;
      }
    },
    [chat, showView],
  );

  /**
   * "Open this report", asked for by the command palette, which lives outside
   * the Reports destination.
   *
   * Read as an intent rather than plumbed as a callback, so the palette needs no
   * handle on the destination. The destination itself consumes the same intent
   * to decide which report to open, and clears it.
   */
  const pendingReportId = useSyncExternalStore(subscribePendingReport, getPendingReportId, getPendingReportId);
  useEffect(() => {
    if (pendingReportId === null) return;
    showView("reports");
  }, [pendingReportId, showView]);

  useEffect(() => {
    if (!paletteOpen) return;
    let live = true;
    const reportRequest = beginReportListRequest();
    void reportList()
      .then((result) => {
        if (live && isCurrentReportListRequest(reportRequest)) {
          applyReportList(result.reports);
        }
      })
      .catch(() => {
        /* A failed prefetch must not invent an empty library. */
      });
    void skillList()
      .then((result) => setPaletteSkills(result.skills))
      .catch(() => setPaletteSkills([]));
    return () => {
      live = false;
      cancelReportListRequest(reportRequest);
    };
  }, [paletteOpen]);

  const startNewChat = useCallback(() => {
    if (!chatReady) return;
    void chat.startNewConversation();
    showView("chat");
  }, [chat, chatReady, showView]);

  /** The destination group's "+": a new chat, project, or dataset upload from any page. */
  const startNew = useCallback(
    (target: WorkspaceView) => {
      if (target === "chat") {
        startNewChat();
        return;
      }
      if (target !== "projects" && target !== "datasets") return;
      goToDestination(target);
      setCreateRequest(target);
    },
    [goToDestination, startNewChat],
  );
  const clearCreateRequest = useCallback(() => setCreateRequest(null), []);

  const clearProjectContext = useCallback(() => {
    void (async () => {
      try {
        await projectDeactivate();
      } catch (cause) {
        toast.error(`Could not leave the project: ${cause instanceof Error ? cause.message : String(cause)}`);
        return;
      }
      await chat.startNewConversation();
      setOpenFile(null);
      await projects.refresh();
      showView("chat");
    })();
  }, [chat, projects, showView]);

  const selectMcpServer = chat.selectMcpServer;
  const refreshProjects = projects.refresh;
  const selectChatMcpServer = useCallback(
    (serverId: string | null) => {
      void selectMcpServer(
        serverId,
        activeProjectId !== null
          ? async () => {
              setMcpSelectionChanging(true);
              try {
                await projectDeactivate();
                setOpenFile(null);
                await refreshProjects();
                return true;
              } catch (cause) {
                toast.error(`Could not leave the project: ${cause instanceof Error ? cause.message : String(cause)}`);
                return false;
              } finally {
                setMcpSelectionChanging(false);
              }
            }
          : undefined,
      );
    },
    [activeProjectId, refreshProjects, selectMcpServer],
  );

  const openConversation = useCallback(
    (conversationId: string, query = "") => {
      setJumpQuery(query);
      showView("chat");
      void (async () => {
        await chat.openConversation(conversationId);
        await projects.refresh();
      })();
      if (isNarrow) setSidebarOpen(false);
    },
    [chat, isNarrow, projects, setSidebarOpen, showView],
  );

  /**
   * Opens one of a project's own conversations, from that project's tree.
   *
   * The same conversation, shown on the destination the reader is standing on.
   * Routing this through `openConversation` sent them to Chats, which took the
   * project tree off the screen as the price of reading one of its chats -- so
   * the row that looked like navigation behaved like an exit.
   */
  const openProjectConversation = useCallback(
    (conversationId: string) => {
      setProjectPane("chat");
      void (async () => {
        await chat.openConversation(conversationId);
        await projects.refresh();
      })();
      if (isNarrow) setSidebarOpen(false);
    },
    [chat, isNarrow, projects, setSidebarOpen],
  );

  /**
   * Starts a chat in one project, shown where that project's chats are shown.
   *
   * This used to be what pressing a project's NAME did, back when the tree
   * lived in the chat column and "open project" meant "start talking to it".
   * On the Projects destination that was a trapdoor: one press replaced the
   * open conversation with an empty one and swapped the column out from under
   * the reader. It is a button on the project's own row now, and it does not
   * leave: the new chat opens beside the tree, exactly where the project's
   * existing chats open. Sending it to Chats would have been worse than an
   * exit -- Chats lists the loose conversations only, so a brand new project
   * chat would have been on screen and in no list at all.
   */
  const startProjectChat = useCallback(
    (projectId: string) => {
      void (async () => {
        if (!(await projects.activate(projectId))) return;
        await chat.startNewConversation();
        setProjectPane("chat");
        showView("projects");
      })();
    },
    [chat, projects, showView],
  );

  /**
   * Opens one project's file view. Activating first matters: that view reads
   * the *active* project, so opening the row of a project that is not active
   * would otherwise show a different project's files under its name.
   *
   * `path` names a file to open with it — what a sidebar file row does — and is
   * absent when the whole tree is what was asked for.
   */
  const browseProjectFiles = useCallback(
    (projectId: string, path?: string) => {
      void (async () => {
        if (projectId !== activeProject?.id && !(await projects.activate(projectId))) return;
        setOpenFile(path == null ? null : { projectId, path });
        showProjectFiles();
        showView("projects");
      })();
    },
    [activeProject?.id, projects, showProjectFiles, showView],
  );

  /** `openFile` outlives the render that made it -- a save notice's "Open file"
   * is pressed seconds later -- so it reaches the current opener, whose idea of
   * the active project is today's, not the one from when the save started. */
  const browseProjectFilesRef = useRef(browseProjectFiles);
  browseProjectFilesRef.current = browseProjectFiles;

  /**
   * Where a dashboard in the open conversation saves: its bound project's
   * folder, or nowhere -- with no binding there is no save control, because a
   * server report is created by the assistant. Taken from the conversation's
   * own binding, never the sidebar selection, which can point at a different
   * project than the chat on screen.
   */
  const reportDestination = useMemo<ProjectReportDestination | null>(
    () =>
      chat.conversationId == null || conversationProject == null
        ? null
        : {
            conversationId: chat.conversationId,
            projectId: conversationProject.id,
            projectName: conversationProject.display_name,
            openFile: (path) => browseProjectFilesRef.current(conversationProject.id, path),
          },
    [chat.conversationId, conversationProject],
  );

  /**
   * The projects the Reports library can copy a report into: every trusted project
   * whose folder is reachable, the one the reader is working in first. A project
   * whose folder is gone is not offered, because the copy would fail.
   */
  const reportProjects = useMemo<ReportProjectChoices>(
    () => ({
      projects: projects.projects
        .filter((project) => project.status === "trusted")
        .map((project) => ({ id: project.id, name: project.display_name })),
      activeProjectId: activeProject?.id ?? null,
      openFile: (projectId, path) => browseProjectFilesRef.current(projectId, path),
    }),
    [projects.projects, activeProject?.id],
  );

  /**
   * Withdraws trust from a project (`project_revoke`): it leaves the sidebar
   * and the agent loses its files. Nothing on disk is touched.
   *
   * The refresh afterwards is not optional. Revoking clears the backend's
   * active project when it was the one removed, so a sidebar that kept its own
   * copy would go on showing a project that no longer exists and offering to
   * open files inside it. Projects stays the destination either way: the tree
   * you removed it from is still the thing you are working in, and the view
   * beside it says plainly that no project is open.
   */
  const removeProject = useCallback(
    async (projectId: string) => {
      await projectRevoke(projectId);
      if (openFile?.projectId === projectId) setOpenFile(null);
      projects.forget(projectId);
      void projects.refresh();
    },
    [openFile?.projectId, projects],
  );

  /**
   * Renames the open conversation. The refresh is what puts the new name on
   * its sidebar row -- the row comes from the workspace bootstrap, not from
   * the open conversation's own state, so without it the titlebar and the
   * sidebar would disagree until the next reload.
   *
   * Reports whether it succeeded, so every in-place editor can preserve the
   * person's typed title and retry state when the write fails.
   */
  const renameConversationById = useCallback(
    async (conversationId: string, title: string): Promise<boolean> => {
      try {
        await chatConversationRename(conversationId, title);
        await chat.refreshWorkspace();
      } catch (cause) {
        toast.error(cause instanceof Error ? cause.message : String(cause));
        return false;
      }
      return true;
    },
    [chat],
  );

  /** The titlebar renames whatever is open, so it supplies the id itself. */
  const renameConversation = useCallback(
    async (title: string): Promise<boolean> => {
      if (chat.conversationId == null) return false;
      return renameConversationById(chat.conversationId, title);
    },
    [chat.conversationId, renameConversationById],
  );

  const refreshWorkspace = useCallback(() => {
    void workspace.refresh();
    void chat.refreshWorkspace();
  }, [chat, workspace]);

  /** Resolves to whether the session really ended; a logout that leaves it live is not a sign-out. */
  const signOut = useCallback(async (): Promise<boolean> => {
    const next = await authLogout();
    setAuthSession(next);
    return next.status !== "authenticated";
  }, []);

  // A run that is still going is work the person started, so an unattended
  // window does not sign out under it; the idle period restarts when it ends.
  const runInProgress = useChatSelector(
    (snapshot) =>
      snapshot.awaitingRun || snapshot.revising || isBoundRunPending(snapshot.activeRunId, snapshot.runState),
  );
  useIdleSignOut({
    busy: runInProgress,
    // An open dialog or sheet holds input the person has not saved; signing out would unmount it.
    isBusy: () => document.querySelector('[role="dialog"], [role="alertdialog"]') !== null,
    onIdle: async () => {
      let signedOut = false;
      try {
        signedOut = await signOut();
      } catch {
        // Reported below: the hook retries in five minutes, and the session stays live until then.
      }
      if (!signedOut) {
        toast.error(
          "Could not sign you out after 12 hours of inactivity. Close xFalcon or sign out from the account menu.",
        );
        return false;
      }
      // The shell unmounts on sign-out, so the notice goes where the sign-in screen will read it.
      setSignedOutNotice(idleSignOutNotice());
      return true;
    },
  });

  const previousSidebarState = useRef(sidebarOpen);

  const toggleZenMode = useCallback(() => {
    setSidebarOpen((currentOpen) => {
      if (currentOpen) {
        previousSidebarState.current = true;
        return false;
      } else {
        return previousSidebarState.current;
      }
    });
  }, [setSidebarOpen]);

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen(true);
      }
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "b") {
        event.preventDefault();
        toggleSidebar();
      }
      // Zen / Focus Mode (Cmd+Shift+F / Ctrl+Shift+F)
      if ((event.metaKey || event.ctrlKey) && event.shiftKey && event.key.toLowerCase() === "f") {
        event.preventDefault();
        toggleZenMode();
      }
      // Shortcuts cheat sheet (Cmd+/ / Ctrl+/)
      if ((event.metaKey || event.ctrlKey) && event.key === "/") {
        event.preventDefault();
        setShortcutHelpOpen((prev) => !prev);
      }
    };
    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  }, [toggleSidebar, toggleZenMode]);

  // Workspace arrival (spec: workspace-arrival): hold a whole-shell skeleton
  // between "signed in" and "workspace bootstrap landed", so the reveal shows
  // complete data instead of a shell that populates piecemeal. The cap keeps
  // a hung bootstrap from holding the app hostage; setTimeout only (frontend
  // privilege boundary bans setInterval).
  // Outside the Tauri shell (jsdom, plain browser) the bootstrap can never
  // run — `useChat.refreshWorkspace` returns before fetching — so the
  // skeleton must not wait on a load that will never start.
  /**
   * `workspace.status` itself starts `null` and only resolves after a round
   * trip (`useWorkspaceConnection`'s own effect), so `chatReady` reads
   * `false` for that first render whether the tenant is actually not ready or
   * the answer just has not arrived yet. Gating the skeleton on `chatReady`
   * alone conflated those two: sign-in landed on a shell with an empty
   * sidebar and titlebar for that round trip, then the skeleton cut in once
   * the status caught up, then the real shell replaced it -- the exact
   * partial-app flash this skeleton exists to prevent. `statusPending` covers
   * that gap; once the status call has answered (either with data or with an
   * error), `chatReady` is the tenant's real, settled answer again and a
   * permanently-not-ready tenant still falls through to the real shell rather
   * than hanging here forever.
   *
   * Latched to the FIRST resolution only. `workspace.refresh` (wired to
   * Settings' `onWorkspaceChanged`, fired after adding a provider credential
   * or editing the MCP registry -- the exact repair a person reaches for
   * after landing on a not-ready tenant) clears `error` before its new
   * fetch lands, but never touches `status`. Read unlatched, that reopens
   * this same `status == null && error == null` window on every later
   * refresh, and unlike the first one it fires mid-session: a person deep in
   * Settings would be yanked out to a full-shell skeleton, losing their
   * place, for however long the refetch took. The first arrival is the only
   * one this skeleton owns; every refresh after it is the settled shell's
   * problem, not the arrival gate's.
   */
  const hasResolvedWorkspaceOnce = useRef(false);
  if (workspace.status != null || workspace.error != null) hasResolvedWorkspaceOnce.current = true;
  const statusPending = !hasResolvedWorkspaceOnce.current;
  const awaitingWorkspace = identity != null && inTauriShell && (statusPending || (chatReady && !chat.workspaceLoaded));
  const [arrivalCapExpired, setArrivalCapExpired] = useState(false);
  useEffect(() => {
    if (!awaitingWorkspace) return;
    const cap = window.setTimeout(() => setArrivalCapExpired(true), WORKSPACE_ARRIVAL_CAP_MS);
    return () => window.clearTimeout(cap);
  }, [awaitingWorkspace]);
  useEffect(() => {
    // Next sign-in replays the skeleton with a fresh cap.
    if (identity == null) setArrivalCapExpired(false);
  }, [identity]);

  // Once the shell is on screen, fetch the lazy views in the background so the
  // first visit to one does not wait on its chunk. The latch is set when the
  // timer fires, not when it is scheduled, so the preload runs once per mount
  // even when an effect re-run (Strict Mode) cancels the first timer.
  const shellInteractive = identity != null && !(awaitingWorkspace && !arrivalCapExpired);
  const viewsPreloaded = useRef(false);
  useEffect(() => {
    if (!shellInteractive || viewsPreloaded.current) return;
    const preload = window.setTimeout(() => {
      viewsPreloaded.current = true;
      preloadWorkspaceViews();
    }, 300);
    return () => window.clearTimeout(preload);
  }, [shellInteractive]);

  // A new array per render would make CommandPalette rescan the transcript on
  // every render; keep it until the messages change.
  const messageTexts = useMemo(() => chat.messages.map((message) => message.text), [chat.messages]);

  if (awaitingWorkspace && !arrivalCapExpired) {
    // The skeleton IS the shell here, not a child of it: `.app-shell` is a
    // three-track grid, so a lone child lands in the sidebar track and the
    // whole skeleton renders squeezed into 17rem.
    return <WorkspaceSkeleton />;
  }

  return (
    <div
      className="app-shell"
      data-canvas-expanded={canvasOpen && canvasState.isExpanded}
      data-canvas-open={canvasOpen}
      data-sidebar-open={sidebarOpen}
    >
      <aside
        aria-hidden={sidebarHidden}
        className="app-sidebar"
        data-rail={railed}
        id="app-sidebar"
        inert={sidebarHidden}
      >
        {railed ? (
          <SidebarRail
            account={
              <AccountMenu
                applicationVersion={bootstrap.application_version}
                identity={identity}
                platform={bootstrap.platform}
                variant="compact"
                onSignOut={() => void signOut()}
              />
            }
            view={selectedDestination}
            newChatDisabled={!chatReady}
            onNavigate={goToDestination}
            onNewChat={startNewChat}
            windowDrag={windowDrag}
          />
        ) : (
          <nav aria-label="Workspace" className="app-sidebar__content">
            <div className="app-sidebar__brand" data-tauri-drag-region={windowDrag ? "deep" : undefined}>
              <Logo />
            </div>

            {/* Where you are, and everywhere else you can be, in one group at
                the top of the column -- so the answer never depends on how far
                the recents list has scrolled. */}
            <DestinationNav
              chatReady={chatReady}
              view={selectedDestination}
              onNavigate={goToDestination}
              onCreate={startNew}
            />

            <div className="app-sidebar__body">
              {/* The two lists share the available height. Utility destinations
                stay in stable chrome below them instead of each claiming a
                fixed percentage and leaving empty bands. */}
              <div className="app-sidebar__explore">
                {/* Chats owns everything you do with chats: starting one and
                    the recent ones. Projects owns the tree, and each of the two
                    fills the column on its own -- neither is drawn on the
                    other's destination, and neither leaves a band of empty
                    column reserved for the one that is not there. */}
                {chatSurface ? (
                  <div className="app-sidebar__section app-sidebar__section--chat app-sidebar__section--fill">
                    <div className="sidebar-chat-children">
                      <button
                        className="nav-item nav-item--child nav-item--create w-full border-0 bg-transparent"
                        disabled={!chatReady}
                        type="button"
                        onClick={startNewChat}
                      >
                        <MessageSquarePlus aria-hidden size={16} strokeWidth={1.8} />
                        <span>New chat</span>
                      </button>
                    </div>

                    <div className="app-sidebar__scroll">
                      <ConversationList
                        activeConversationId={chat.conversationId}
                        conversations={recentConversations}
                        disabled={!chatReady}
                        onDelete={requestChatDelete}
                        onPin={(conversationId, pinned) => {
                          void chat.pinConversation(conversationId, pinned);
                        }}
                        onRename={renameConversationById}
                        onSelect={openConversation}
                      />
                    </div>
                  </div>
                ) : null}

                {projectSurface ? (
                  <div className="app-sidebar__section app-sidebar__section--fill">
                    <ProjectNavList
                      createRequested={createRequest === "projects"}
                      onCreateHandled={clearCreateRequest}
                      activeConversationFiles={activeConversationFiles}
                      activeConversationId={chat.conversationId}
                      activeProjectId={activeProject?.id}
                      chatReady={chatReady}
                      conversationsFor={conversationsFor}
                      openFilePath={openFilePath}
                      pane={projectPane}
                      projects={projects.projects}
                      projectStatus={projectStatus}
                      onBrowseFiles={browseProjectFiles}
                      onDeleteConversation={requestChatDelete}
                      onOpenConversation={openProjectConversation}
                      onOpenFile={browseProjectFiles}
                      onPinConversation={(conversationId, pinned) => {
                        void chat.pinConversation(conversationId, pinned);
                      }}
                      onRenameConversation={renameConversationById}
                      /* Creation and reuse both activate the project in the backend. */
                      onProjectReady={() => {
                        showProjectFiles();
                        setOpenFile(null);
                        void projects.refresh();
                        if (isNarrow) setSidebarOpen(false);
                      }}
                      onRemoveProject={removeProject}
                      onStartProjectChat={startProjectChat}
                    />
                    {projects.error ? (
                      <p className="type-chrome text-danger" role="alert">
                        {projects.error}
                      </p>
                    ) : null}
                  </div>
                ) : null}
              </div>

              <div className="app-sidebar__utilities">
                <div className="app-sidebar__section">
                  <div className="relative">
                    <button
                      aria-label={SETTINGS_DESTINATION.label}
                      aria-describedby="settings-update-count"
                      aria-current={view === "settings" ? "page" : undefined}
                      className={cn(
                        "nav-item nav-item--title nav-item--destination w-full border-0 bg-transparent",
                        view === "settings" && "active",
                      )}
                      type="button"
                      onClick={() => goToDestination(SETTINGS_DESTINATION.view)}
                    >
                      <SettingsIcon aria-hidden size={16} strokeWidth={1.8} />
                      <span>{SETTINGS_DESTINATION.label}</span>
                    </button>
                    <SettingsUpdateBadge
                      onOpenUpdates={() => {
                        setSettingsFocus({
                          ...settingsSectionReveal("application"),
                          targetId: "settings-updates",
                          token: Date.now(),
                        });
                        showView("settings");
                      }}
                    />
                  </div>

                  <AccountMenu
                    applicationVersion={bootstrap.application_version}
                    identity={identity}
                    platform={bootstrap.platform}
                    onSignOut={() => void signOut()}
                  />
                </div>
              </div>
            </div>
          </nav>
        )}
        {/* Last, so Tab reaches the sidebar's own controls before the resize handle. */}
        {sidebarOpen && !isNarrow ? <SidebarResizer /> : null}
      </aside>

      {isNarrow && sidebarOpen ? (
        <button
          aria-label="Close sidebar"
          className="app-sidebar-scrim"
          type="button"
          onClick={() => setSidebarOpen(false)}
        />
      ) : null}

      <main
        aria-label={projectSurface && projectPane === "chat" ? "Project chat" : MAIN_LABEL[view]}
        className="app-workspace"
      >
        <OfflineBanner visible={!online} />
        <header className="app-titlebar" data-tauri-drag-region={windowDrag ? "deep" : undefined}>
          <Tooltip label={sidebarOpen ? "Hide sidebar" : "Show sidebar"} side="bottom">
            <IconButton
              aria-controls="app-sidebar"
              aria-expanded={sidebarOpen}
              aria-label={sidebarOpen ? "Hide sidebar" : "Show sidebar"}
              className="touch-target border border-transparent hover:border-default hover:bg-card"
              type="button"
              onClick={toggleSidebar}
            >
              {sidebarOpen ? <PanelLeftClose aria-hidden size={18} /> : <PanelLeftOpen aria-hidden size={18} />}
            </IconButton>
          </Tooltip>
          <div className="app-titlebar__identity">
            {/* Only a conversation can be renamed here -- Reports and Settings
                title themselves, and a project is named where it is trusted. */}
            {chatPaneVisible ? (
              chat.conversationId != null ? (
                <ConversationTitle
                  conversationId={chat.conversationId}
                  onRename={renameConversation}
                  title={chat.title}
                  subtitle={titlebarSubtitle ?? undefined}
                />
              ) : (
                // A transcript names itself on whichever destination is
                // showing it. Falling through to the destination's own name
                // titled an unsaved project chat after the project, which the
                // subtitle underneath already says.
                <strong>{chat.title}</strong>
              )
            ) : (
              <strong>
                {TITLEBAR_NAME[view]({
                  conversationTitle: chat.title,
                  projectName: activeProject?.display_name ?? null,
                })}
              </strong>
            )}
            {/* On Projects the subtitle says which of the project's two things
                is on screen; the title above it is already the project's name,
                or the conversation's. */}
            {titlebarSubtitle && !(chatPaneVisible && chat.conversationId != null) ? (
              <TruncatedText>{titlebarSubtitle}</TruncatedText>
            ) : null}
          </div>
          <div className="app-titlebar__actions flex items-center gap-2">
            {/* The one way into search besides the shortcut. The sidebar used to
                carry a second button that opened the same palette. */}
            <button
              aria-label="Search"
              className="app-command-button touch-target press"
              type="button"
              onClick={() => setPaletteOpen(true)}
            >
              <Search aria-hidden size={15} />
              <span>Search</span>
              <Shortcut keys={COMMAND_PALETTE_KEYS} />
            </button>
            {chatPaneVisible && chat.messages.length > 0 ? (
              <Tooltip label="Export this chat" side="bottom">
                <button
                  aria-label="Export chat"
                  className="app-command-button touch-target press"
                  type="button"
                  onClick={() => setExportOpen(true)}
                >
                  <FileDown aria-hidden size={15} />
                  <span>Export chat</span>
                </button>
              </Tooltip>
            ) : null}
            {canvasState.tabs.length > 0 && (
              <Tooltip
                label={canvasOpen ? "Hide the artifact side panel" : "Show the artifact side panel"}
                side="bottom"
              >
                <IconButton
                  aria-expanded={canvasOpen}
                  aria-label={canvasOpen ? "Hide side panel" : "Show side panel"}
                  className="touch-target border border-transparent hover:border-default hover:bg-card"
                  type="button"
                  onClick={toggleCanvasOpen}
                >
                  {canvasOpen ? <PanelRightClose aria-hidden size={17} /> : <PanelRightOpen aria-hidden size={17} />}
                </IconButton>
              </Tooltip>
            )}
          </div>
        </header>
        <div className="app-workspace__body">
          <UpdateNotice centered={view === "settings"} />

          {/* One boundary for the views, mounted for the whole session. A view
              change is a transition, and React keeps the outgoing view on screen
              while the incoming one loads only when the boundary it suspends in
              was already on screen. A boundary created by the new view would
              reveal its empty fallback at once and drop the outgoing view. */}
          <Suspense fallback={null}>
            {view === "settings" ? (
              <SettingsView mcpRevision={mcpRevision} reveal={settingsFocus} onWorkspaceChanged={refreshWorkspace} />
            ) : null}

            {view === "datasets" ? (
              <DatasetsView pickRequested={createRequest === "datasets"} onPickHandled={clearCreateRequest} />
            ) : null}
            {view === "reports" ? (
              <ReportProjectsContext.Provider value={reportProjects}>
                <ReportsView active={view === "reports"} revealQuery={jumpQuery} />
              </ReportProjectsContext.Provider>
            ) : null}

            {projectSurface && projectPane === "files" ? (
              <ProjectExplorerView
                onProjectReady={() => {
                  setProjectPane("files");
                  setOpenFile(null);
                  void projects.refresh();
                  if (isNarrow) setSidebarOpen(false);
                }}
                onSelectPath={(path) => {
                  if (activeProject != null) setOpenFile({ projectId: activeProject.id, path });
                }}
                project={activeProject ?? null}
                revealQuery={jumpQuery}
                selectedPath={openFilePath}
              />
            ) : null}
          </Suspense>

          {/* Unmounted rather than hidden when another view is up: every scrap of
              conversation state lives in `useChat` above, and `.app-chat-column`'s
              own `display: flex` would win over the `hidden` attribute anyway. */}
          {!chatPaneVisible ? null : (
            <ReportDestinationContext.Provider value={reportDestination}>
              <ChatPane
                activeProjectName={activeChatProjectName}
                disabled={!chatReady}
                onClearProject={clearProjectContext}
                projectUnavailable={activeChatProjectUnavailable}
                viewerName={identity?.display_name ?? null}
                onDismissNotice={chat.dismissNotice}
                onOpenSettings={() => {
                  setSettingsFocus(null);
                  showView("settings");
                }}
                onResetConversation={() => void chat.startNewConversation()}
                onRetryRun={() => {
                  void chat.retryLastRun();
                }}
                onStop={() => {
                  void chat.cancelRun();
                }}
                stopping={chat.stopping}
                onSubmit={(prompt, attachmentIds) => {
                  void chat.send(prompt, attachmentIds);
                }}
                onAttach={() => {
                  void chat.attachFile();
                }}
                onPasteFiles={chat.pasteFiles}
                attachmentPending={chat.pastingFiles}
                onCancelUpload={(uploadId) => {
                  void chat.cancelUpload(uploadId);
                }}
                onRetryUpload={(uploadId) => {
                  void chat.retryUpload(uploadId);
                }}
                onEdit={(messageId, text) => {
                  void chat.editTurn(messageId, text);
                }}
                onRegenerate={(messageId) => {
                  void chat.regenerateTurn(messageId);
                }}
                mcpServers={mcpServers}
                mcpServerId={chat.mcpSelection.serverId}
                mcpSelectionLoading={chat.mcpSelection.status === "loading"}
                mcpSelectionError={chat.mcpSelection.error}
                mcpSelectionDisabled={
                  chat.attachingFile || mcpSelectionChanging || chat.mcpSelection.status === "error"
                }
                onMcpServerSelect={selectChatMcpServer}
                onSlashCommand={handleSlashCommand}
                projectFiles={projectFiles}
                skills={paletteSkills
                  .filter((skill) => skill.enabled)
                  .map((skill) => ({
                    name: skill.name,
                    description: skill.description,
                  }))}
              />
            </ReportDestinationContext.Provider>
          )}
        </div>
      </main>

      <CanvasSlot open={canvasOpen} expanded={canvasState.isExpanded} ariaLabel="Artifact side panel">
        <Suspense fallback={null}>
          <CanvasPanel />
        </Suspense>
      </CanvasSlot>

      <AlertDialog
        open={pendingChatDelete !== null}
        title="Delete chat?"
        description={`“${pendingChatDelete?.title || "Untitled conversation"}” will be permanently deleted. This cannot be undone.`}
        confirmLabel="Delete chat"
        destructive
        busy={chatDeleteBusy}
        error={chatDeleteError}
        onCancel={() => {
          if (chatDeleteInFlight.current) return;
          setPendingChatDelete(null);
          setChatDeleteError(null);
        }}
        onConfirm={() => void confirmChatDelete()}
      />
      {/* Reads the transcript the reader is looking at rather than re-fetching
          it: reasoning exists only in the chat store, so a backend-composed
          export could never carry the "include reasoning" choice. Mounted only
          while open — see `ChatExportDialog`. */}
      {exportOpen ? (
        <Suspense fallback={null}>
          <ChatExportDialog onClose={closeExport} />
        </Suspense>
      ) : null}

      <CommandPalette
        conversations={chat.conversations}
        projects={projects.projects}
        reports={paletteReports}
        mcpServers={workspace.status?.mcp_servers ?? []}
        skills={paletteSkills}
        open={paletteOpen}
        openConversationId={chat.conversationId}
        openMessageTexts={messageTexts}
        searchConversationMessages={chatConversationSearch}
        onBrowseFiles={() => showView("projects")}
        onClose={() => setPaletteOpen(false)}
        onGoToChat={() => showView("chat")}
        onNewChat={startNewChat}
        onOpenConversation={openConversation}
        onOpenProject={(projectId, query) => {
          setJumpQuery(query);
          browseProjectFiles(projectId);
        }}
        onOpenReport={(reportId, query) => {
          setJumpQuery(query);
          requestOpenReport(reportId);
          showView("reports");
        }}
        onOpenDatasets={() => showView("datasets")}
        onOpenReports={() => showView("reports")}
        onOpenSettings={() => {
          setSettingsFocus(null);
          showView("settings");
        }}
        onOpenSettingsSection={(reveal) => {
          setSettingsFocus({ ...reveal, token: Date.now() });
          showView("settings");
        }}
      />

      <ShortcutHelpDialog open={shortcutHelpOpen} onClose={() => setShortcutHelpOpen(false)} />

      {/* App-wide on purpose: a server refuses its sign-in during a run, which
          is never the moment the user happens to have Settings open. */}
      <McpReauthPrompt
        onReconnected={() => {
          setMcpRevision((revision) => revision + 1);
          refreshWorkspace();
        }}
      />
    </div>
  );
}
