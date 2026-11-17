import type { Page } from "@playwright/test";
import type {
  InferenceEvent,
  McpServerSummary,
  ProjectDirectoryListing,
  ProjectView,
} from "../../src/lib/generated/contracts";

export const chatE2eBootstrap = {
  version: 1,
  application_version: "0.1.0-e2e-test",
  platform: "windows",
  session: {
    status: "authenticated",
    version: 1,
    identity: {
      version: 1,
      user_id: "e2e-user",
      tenant_id: "e2e-tenant",
      display_name: "E2E Test",
      email: null,
    },
    expires_at_ms: Date.now() + 3_600_000,
  },
} as const;

export const chatE2eWorkspaceStatus = {
  version: 1,
  tenant_host: "tenant.example",
  mcp_resource_url: "https://tenant.example/mcp",
  mcp_servers: [],
  models_available: 1,
  default_model_id: "gpt-test",
  default_provider: "openai",
  chat_ready: true,
  setup_note: "Bundled MCP is automatic.",
} as const;

export type ChatMockOptions = {
  feedbackRatings?: Record<string, number>;
  followUpSuggestions?: string[];
  appearanceTheme?: "light" | "dark" | "system";
  /** Display name returned by the native bootstrap boundary. */
  displayName?: string;
  recoveryNotice?: string | null;
  chatReady?: boolean;
  /** Configured MCP servers returned by the real workspace-status consumer. */
  mcpServers?: McpServerSummary[];
  projects?: ProjectView[];
  projectDirectories?: ProjectDirectoryListing[];
  /** Availability returned by conversation views (`chat_conversation_open`, create, etc.). */
  conversationAvailability?: "live" | "stale";
  /** Seed `chat_workspace_bootstrap` with recents for offline / stale flows. */
  bootstrapConversations?: Array<{
    conversation_id: string;
    title: string;
    updated_at?: string;
    project_id?: string | null;
    pinned_at?: string | null;
  }>;
  /** Messages returned when a seeded conversation is opened after restart. */
  conversationMessages?: Array<{
    message_id: string;
    role: "user" | "assistant" | "system" | "tool";
    text: string;
    sequence: number;
  }>;
  /** In-flight run returned with a seeded conversation view. */
  conversationActiveRunId?: string | null;
  /** Per-conversation run ownership for live navigation checks. */
  conversationActiveRunIds?: Record<string, string | null>;
  /** Never resolve `chat_message_send` after emitting a running run (slice pending UI). */
  pendingSend?: boolean;
  /** Emit a running run event when a message is sent (Path B, no live DA). */
  emitRunningOnSend?: boolean;
  /** After `emitRunningOnSend`, emit `interrupted` so recovery retry is actionable (`activeRunId` set). */
  emitInterruptedAfterRunning?: boolean;
};

export type E2EChatMockInvoke = {
  command: string;
  args: Record<string, unknown>;
};

/** Read invoke log from the page after `installChatMocks`. */
export async function getChatMockInvocations(page: Page, command?: string): Promise<E2EChatMockInvoke[]> {
  return page.evaluate((filter) => {
    const log = (window as Window & { __E2E_CHAT_MOCK__?: { invocations: E2EChatMockInvoke[] } }).__E2E_CHAT_MOCK__
      ?.invocations;
    if (!log) return [];
    return filter ? log.filter((entry) => entry.command === filter) : log;
  }, command);
}

type ChatMockInitPayload = {
  bootstrap: typeof chatE2eBootstrap;
  workspace: typeof chatE2eWorkspaceStatus;
  options: ChatMockOptions;
};

export async function installChatMocks(page: Page, options: ChatMockOptions = {}): Promise<void> {
  const reconfigured = await page.evaluate((nextOptions) => {
    const mock = (window as Window & { __E2E_CHAT_MOCK__?: unknown }).__E2E_CHAT_MOCK__;
    if (!mock) {
      return false;
    }
    window.sessionStorage.setItem("__E2E_CHAT_MOCK_OPTIONS__", JSON.stringify(nextOptions));
    return true;
  }, options);
  if (reconfigured) {
    return;
  }

  const payload: ChatMockInitPayload = {
    bootstrap: chatE2eBootstrap,
    workspace: chatE2eWorkspaceStatus,
    options,
  };

  await page.addInitScript((init: ChatMockInitPayload) => {
    const CHAT_RUN_EVENT_CHANNEL = "xfalcon://chat-run-event";
    const CHAT_PROJECTION_EVENT_CHANNEL = "xfalcon://chat-projection-event";
    const CHAT_INFERENCE_EVENT_CHANNEL = "xfalcon://chat-inference-event";
    const { bootstrap, workspace } = init;
    let persistedOptions: ChatMockOptions = {};
    try {
      const encoded = window.sessionStorage.getItem("__E2E_CHAT_MOCK_OPTIONS__");
      if (encoded) {
        persistedOptions = JSON.parse(encoded) as ChatMockOptions;
      }
    } catch {
      persistedOptions = {};
    }
    const mockOptions = { ...init.options, ...persistedOptions };
    const recoveryNotice = mockOptions.recoveryNotice ?? null;
    const appearanceTheme = mockOptions.appearanceTheme ?? "system";
    const chatReady = mockOptions.chatReady ?? true;
    const mcpServers = mockOptions.mcpServers ?? workspace.mcp_servers;
    const conversationAvailability = mockOptions.conversationAvailability ?? "live";
    const bootstrapConversations = mockOptions.bootstrapConversations ?? [];
    const conversationMessages = mockOptions.conversationMessages ?? [];
    const conversationActiveRunId = mockOptions.conversationActiveRunId ?? null;
    const conversationActiveRunIds = mockOptions.conversationActiveRunIds ?? {};
    const pendingSend = mockOptions.pendingSend ?? false;
    const emitRunningOnSend = mockOptions.emitRunningOnSend ?? false;
    const emitInterruptedAfterRunning = mockOptions.emitInterruptedAfterRunning ?? false;

    const invocations: { command: string; args: Record<string, unknown> }[] = [];

    const listeners = new Map<string, number[]>();
    const callbacks = new Map<number, (data: unknown) => void>();
    let conversationId = "conv-e2e-1";
    let nextRunId = 1;

    function conversationView(conversationIdValue: string, title = "New conversation") {
      const activeRunId = conversationActiveRunIds[conversationIdValue];
      return {
        version: 1,
        conversation_id: conversationIdValue,
        title,
        availability: conversationAvailability,
        messages: conversationMessages.map((message) => ({
          version: 1,
          ...message,
          sequence: BigInt(message.sequence),
          tool_calls: [],
        })),
        active_run_id: activeRunId === undefined ? conversationActiveRunId : activeRunId,
      };
    }

    function unregisterCallback(id: number) {
      callbacks.delete(id);
    }

    function runCallback(id: number, data: unknown) {
      const callback = callbacks.get(id);
      if (callback) {
        callback(data);
      }
    }

    function registerCallback(callback: (data: unknown) => void, once = false) {
      const identifier = window.crypto.getRandomValues(new Uint32Array(1))[0];
      callbacks.set(identifier, (data) => {
        if (once) {
          unregisterCallback(identifier);
        }
        return callback(data);
      });
      return identifier;
    }

    function emitRunEvent(runId: string, state: string) {
      const runEvent = {
        type: "state_changed",
        version: 1,
        run_id: runId,
        state,
      };
      let attempts = 0;
      const deliver = () => {
        const handlers = listeners.get(CHAT_RUN_EVENT_CHANNEL) ?? [];
        for (const handlerId of handlers) {
          runCallback(handlerId, { event: CHAT_RUN_EVENT_CHANNEL, payload: runEvent });
        }
      };
      const schedule = () => {
        const handlers = listeners.get(CHAT_RUN_EVENT_CHANNEL) ?? [];
        if (handlers.length > 0) {
          deliver();
          return;
        }
        if (attempts >= 400) {
          return;
        }
        attempts += 1;
        window.setTimeout(schedule, 5);
      };
      schedule();
    }

    function emitRunProjection(conversationIdValue: string, runId: string, state: string) {
      const projectionEvent = {
        type: "run_state_changed",
        version: 1,
        conversation_id: conversationIdValue,
        run_id: runId,
        state,
      };
      let attempts = 0;
      const schedule = () => {
        const handlers = listeners.get(CHAT_PROJECTION_EVENT_CHANNEL) ?? [];
        if (handlers.length > 0) {
          for (const handlerId of handlers) {
            runCallback(handlerId, {
              event: CHAT_PROJECTION_EVENT_CHANNEL,
              payload: projectionEvent,
            });
          }
          return;
        }
        if (attempts >= 400) return;
        attempts += 1;
        window.setTimeout(schedule, 5);
      };
      schedule();
    }

    function emitInferenceEvent(event: InferenceEvent) {
      const handlers = listeners.get(CHAT_INFERENCE_EVENT_CHANNEL) ?? [];
      for (const handlerId of handlers) {
        runCallback(handlerId, { event: CHAT_INFERENCE_EVENT_CHANNEL, payload: event });
      }
    }

    function isEventPluginInvoke(cmd: string) {
      return cmd.startsWith("plugin:event|");
    }

    function handleEventPlugin(cmd: string, args: Record<string, unknown>) {
      if (cmd === "plugin:event|listen") {
        const event = args.event as string;
        const handler = args.handler as number;
        if (!listeners.has(event)) {
          listeners.set(event, []);
        }
        listeners.get(event)?.push(handler);
        return handler;
      }
      if (cmd === "plugin:event|emit") {
        const event = args.event as string;
        const eventListeners = listeners.get(event) ?? [];
        for (const handler of eventListeners) {
          runCallback(handler, args);
        }
        return null;
      }
      if (cmd === "plugin:event|unlisten") {
        const event = args.event as string;
        const id = args.eventId as number;
        const eventListeners = listeners.get(event);
        if (eventListeners) {
          const index = eventListeners.indexOf(id);
          if (index !== -1) {
            eventListeners.splice(index, 1);
          }
        }
        return null;
      }
      return null;
    }

    const feedbackRatings = { ...mockOptions.feedbackRatings };

    function invoke(command: string, args: Record<string, unknown> = {}) {
      invocations.push({ command, args: { ...args } });
      if (isEventPluginInvoke(command)) {
        return Promise.resolve(handleEventPlugin(command, args));
      }

      if (command === "app_bootstrap") {
        return Promise.resolve({
          ...bootstrap,
          session: {
            ...bootstrap.session,
            identity: {
              ...bootstrap.session.identity,
              display_name: mockOptions.displayName ?? bootstrap.session.identity.display_name,
            },
          },
        });
      }
      if (command === "app_health") {
        return Promise.resolve({ version: 1, status: "ready" });
      }
      if (command === "workspace_status") {
        return Promise.resolve({ ...workspace, chat_ready: chatReady, mcp_servers: mcpServers });
      }
      if (command === "settings_appearance_get") {
        return Promise.resolve({
          version: 1,
          theme: appearanceTheme,
          reading_size: "small",
          show_reasoning: false,
          show_follow_ups: true,
        });
      }
      if (command === "project_list") {
        return Promise.resolve({ version: 1, projects: mockOptions.projects ?? [] });
      }
      if (command === "project_activate") {
        const project = mockOptions.projects?.find((item) => item.id === args.projectId);
        if (!project) return Promise.reject(new Error("Unknown project"));
        return Promise.resolve({ ...project, active: true });
      }
      if (command === "project_list_directory") {
        const listing = mockOptions.projectDirectories?.find(
          (item) => item.project_id === args.projectId && item.path === args.path,
        );
        if (!listing) return Promise.reject(new Error(`Unknown directory: ${String(args.path)}`));
        return Promise.resolve(listing);
      }
      if (command === "project_file_stat") {
        return Promise.resolve({ version: 1, kind: "text", path: args.path, size_bytes: 19, modified_ms: null });
      }
      if (command === "project_file_read_bytes") {
        return Promise.resolve(new TextEncoder().encode("export default 42;\n").buffer);
      }
      if (command === "project_schema_summary") {
        return Promise.resolve({ version: 1, project_id: args.projectId, tables: [] });
      }
      if (command === "chat_message_feedback_list") {
        return Promise.resolve(Object.entries(feedbackRatings).map(([message_id, rating]) => ({ message_id, rating })));
      }
      if (command === "chat_message_feedback_set") {
        const message_id = args.messageId as string;
        if (args.rating === null) {
          delete feedbackRatings[message_id];
          return Promise.resolve(null);
        }
        feedbackRatings[message_id] = args.rating as number;
        return Promise.resolve({ message_id, rating: args.rating });
      }
      if (command === "chat_follow_ups") {
        return Promise.resolve({ suggestions: mockOptions.followUpSuggestions ?? [] });
      }
      if (command === "chat_workspace_bootstrap") {
        return Promise.resolve({
          version: 1,
          conversations: bootstrapConversations.map((conversation) => ({
            version: 1,
            conversation_id: conversation.conversation_id,
            title: conversation.title,
            updated_at: conversation.updated_at ?? new Date().toISOString(),
            project_id: conversation.project_id ?? null,
            pinned_at: conversation.pinned_at ?? null,
          })),
          recovery_notice: recoveryNotice,
        });
      }
      if (command === "chat_conversation_open") {
        const conversationIdValue = (args.conversationId as string) ?? conversationId;
        const summary = bootstrapConversations.find((c) => c.conversation_id === conversationIdValue);
        return Promise.resolve(conversationView(conversationIdValue, summary?.title ?? "Cached conversation"));
      }
      if (command === "chat_conversation_create") {
        conversationId = `conv-e2e-${Date.now()}`;
        const openingMessage = typeof args.openingMessage === "string" ? args.openingMessage.trim() : "";
        return Promise.resolve(conversationView(conversationId, openingMessage || "New conversation"));
      }
      if (command === "chat_conversation_reset_active") {
        return Promise.resolve(undefined);
      }
      if (command === "chat_run_cancel") {
        return Promise.resolve(undefined);
      }
      if (command === "chat_run_retry") {
        const runId = (args.runId as string) ?? `run-e2e-${nextRunId++}`;
        return Promise.resolve({
          version: 1,
          status: "submitted",
          conversation_id: conversationId,
          run_id: runId,
          message: "Retry accepted by mock tenant.",
        });
      }
      if (command === "app_reset") {
        return Promise.resolve(undefined);
      }
      if (command === "chat_message_send") {
        const runId = `run-e2e-${nextRunId++}`;
        if (!args.conversationId) {
          conversationId = `conv-e2e-${Date.now()}`;
        }
        if (emitRunningOnSend) {
          emitRunProjection(conversationId, runId, "running");
          if (emitInterruptedAfterRunning) {
            window.setTimeout(() => emitRunEvent(runId, "interrupted"), 25);
          }
        }
        const result = {
          version: 1,
          status: "submitted",
          conversation_id: conversationId,
          run_id: runId,
          message: "Run accepted by mock tenant.",
        };
        if (pendingSend) {
          return new Promise(() => {});
        }
        return Promise.resolve(result);
      }
      return Promise.reject(new Error(`e2e chat mock rejected command: ${command}`));
    }

    Object.defineProperty(window, "__TAURI_EVENT_PLUGIN_INTERNALS__", {
      configurable: true,
      value: {
        unregisterListener(_event: string, id: number) {
          unregisterCallback(id);
        },
      },
      writable: true,
    });

    Object.defineProperty(window, "__E2E_CHAT_MOCK__", {
      configurable: true,
      value: {
        invocations,
        emitInferenceEvent,
        emitRunEvent,
        emitRunProjection,
      },
      writable: true,
    });

    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: false,
      value: {
        invoke,
        transformCallback: registerCallback,
        unregisterCallback,
        runCallback,
        callbacks,
      },
      writable: false,
    });
  }, payload);
}
