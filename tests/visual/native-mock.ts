import type { Page } from "@playwright/test";
import type {
  ChatModelCatalogView,
  CustomModelCreateInput,
  CustomModelView,
  McpServerInput,
  McpServerView,
  ProviderCredentialView,
  SkillView,
} from "../../src/lib/generated/contracts";

export const visualBootstrap = {
  version: 1,
  application_version: "0.1.0-visual-test",
  platform: "windows",
  session: {
    status: "authenticated",
    version: 1,
    identity: {
      version: 1,
      user_id: "visual-user",
      tenant_id: "visual-tenant",
      display_name: "Visual Test",
      email: null,
    },
    expires_at_ms: Date.now() + 3_600_000,
  },
} as const;

export type VisualSessionStatus = "authenticated" | "unauthenticated" | "authenticating";

export type VisualNativeMockOptions = {
  theme?: "light" | "dark";
  automaticUpdate?: boolean;
  updatesUpToDate?: boolean;
  releaseNotes?: string | null;
  populatedSidebar?: boolean;
  arrivalPending?: boolean;
  /** Never resolves `app_bootstrap`, leaving App on the loading screen. */
  bootstrapPending?: boolean;
  /** Rejects `app_bootstrap`, leaving App on the startup error screen. */
  bootstrapReject?: boolean;
  sessionStatus?: VisualSessionStatus;
  /** Exercises labels received from the native boundary at their real consumers. */
  longLabels?: boolean;
  mcpRecovery?: boolean;
  modelSettingsReady?: boolean;
  privateModelDiscovery?: "available" | "unsupported";
  providerCredentials?: ProviderCredentialView[];
  skills?: SkillView[];
};

const LONG_LABEL = "Quarterly revenue and customer-retention analysis for international enterprise accounts";

const visualModelCatalog: ChatModelCatalogView = {
  version: 1,
  models: [
    {
      version: 1,
      id: "~anthropic/claude-sonnet-latest",
      provider: "openrouter",
      display_name: "Sonnet",
      capabilities: { tools: true, image_input: true, reasoning_summary: true },
    },
    {
      version: 1,
      id: "~x-ai/grok-latest",
      provider: "openrouter",
      display_name: "Grok",
      capabilities: { tools: true, image_input: false, reasoning_summary: true },
    },
    {
      version: 1,
      id: "~openai/gpt-luna-latest",
      provider: "openrouter",
      display_name: "Luna",
      capabilities: { tools: true, image_input: false, reasoning_summary: true },
    },
    {
      version: 1,
      id: "~openai/gpt-sol-latest",
      provider: "openrouter",
      display_name: "Sol",
      capabilities: { tools: true, image_input: false, reasoning_summary: false },
    },
    {
      version: 1,
      id: "custom/018f0f45-6d2d-7b41-a741-a488b456805a",
      provider: "custom_openai",
      display_name: "QWEN",
      capabilities: { tools: true, image_input: true, reasoning_summary: false },
    },
  ],
  selected_model_id: "custom/018f0f45-6d2d-7b41-a741-a488b456805a",
  selection_locked: false,
};

const visualCustomModels: CustomModelView[] = [
  {
    version: 1,
    id: "018f0f45-6d2d-7b41-a741-a488b456805a",
    model_id: "custom/018f0f45-6d2d-7b41-a741-a488b456805a",
    display_name: "QWEN",
    base_url: "https://models.example.com/v1",
    upstream_model_id: "qwen-3.5",
    context_length: 131_072,
    max_output_tokens: 16_384,
    supports_tools: true,
    supports_vision: true,
    sampling: { temperature: null, top_p: null },
    is_enabled: true,
    is_active: true,
  },
];

function bootstrapWithSession(status: VisualSessionStatus, longLabels: boolean) {
  const authenticated = longLabels
    ? {
        ...visualBootstrap,
        session: {
          ...visualBootstrap.session,
          identity: { ...visualBootstrap.session.identity, display_name: LONG_LABEL },
        },
      }
    : visualBootstrap;
  if (status === "authenticated") return authenticated;
  return {
    ...authenticated,
    session: { status, version: 1 as const },
  };
}

export async function installVisualNativeMock(
  page: Page,
  {
    theme = "light",
    automaticUpdate = false,
    updatesUpToDate = false,
    releaseNotes = null,
    populatedSidebar = false,
    arrivalPending = false,
    bootstrapPending = false,
    bootstrapReject = false,
    sessionStatus = "authenticated",
    longLabels = false,
    mcpRecovery = false,
    modelSettingsReady = false,
    privateModelDiscovery = "available",
    providerCredentials = [],
    skills = [],
  }: VisualNativeMockOptions = {},
): Promise<void> {
  await page.addInitScript(
    ({
      automaticUpdate,
      updatesUpToDate,
      arrivalPending,
      bootstrapPending,
      bootstrapReject,
      mockBootstrap,
      populatedSidebar,
      longLabels,
      mcpRecovery,
      longLabel,
      mockModelCatalog,
      mockCustomModels,
      modelSettingsReady,
      privateModelDiscovery,
      providerCredentials,
      releaseNotes,
      skills,
      theme,
    }) => {
      let appearance = { version: 1, theme, reading_size: "small", show_reasoning: false, show_follow_ups: true };
      let modelCatalog = mockModelCatalog;
      const conversations = populatedSidebar
        ? Array.from({ length: 18 }, (_, index) => ({
            version: 1,
            conversation_id: `conversation-visual-${index}`,
            title: longLabels ? `${longLabel} ${index + 1}` : `Workspace analysis ${index + 1}`,
            updated_at: `2026-08-${String(18 - index).padStart(2, "0")}T12:00:00Z`,
            pinned_at: index < 2 ? `2026-08-${String(18 - index).padStart(2, "0")}T12:30:00Z` : null,
            project_id: null,
          }))
        : [];
      const projects = populatedSidebar
        ? Array.from({ length: 16 }, (_, index) => ({
            version: 1,
            id: `project-visual-${index}`,
            display_name: longLabels ? `${longLabel} project ${index + 1}` : `Research project ${index + 1}`,
            root_path: `C:\\work\\research-${index + 1}`,
            status: "trusted",
            active: index === 0,
            trusted_at_ms: 1_777_777_777_000 + index,
          }))
        : [];
      const workspaceBootstrap = { version: 1, conversations, recovery_notice: null };
      let resolveWorkspaceBootstrap: ((value: typeof workspaceBootstrap) => void) | undefined;
      const pendingWorkspaceBootstrap = arrivalPending
        ? new Promise<typeof workspaceBootstrap>((resolve) => {
            resolveWorkspaceBootstrap = resolve;
          })
        : null;
      if (arrivalPending) {
        Object.defineProperty(window, "__resolveVisualWorkspaceBootstrap", {
          configurable: false,
          value: () => resolveWorkspaceBootstrap?.(workspaceBootstrap),
          writable: false,
        });
      }

      const updateCalls: string[] = [];
      Object.defineProperty(window, "__visualUpdateCalls", { value: updateCalls });
      const previewRegistrationCalls: string[] = [];
      Object.defineProperty(window, "__visualPreviewRegistrationCalls", { value: previewRegistrationCalls });
      const callbacks = new Map<number, (data: unknown) => void>();
      const eventListeners = new Map<string, number[]>();
      let mcpConnected = false;
      const bundledMcpServer = () => ({
        version: 1,
        id: "tenant",
        name: "tenant",
        display_name: "Tenant Data Analyst",
        description: "Your tenant's bundled analytical workspace. Connected automatically after sign-in.",
        transport: "http",
        url: "https://tenant.example/mcp",
        auth_type: "bearer",
        permission_mode: "ask",
        enabled: true,
        builtin: true,
        slug: "tenant",
        unavailable: null,
        sign_in: null,
        // The bundled connection rides the signed-in session, so it has no
        // client of its own.
        oauth_client_id: null,
        oauth_client_confidential: false,
      });
      const mcpServer = () => ({
        version: 1,
        id: "srv-expired",
        name: "analytics",
        display_name: "Analytics",
        description: "Analytics tools",
        transport: "http",
        url: "https://mcp.example/mcp",
        auth_type: "oauth",
        permission_mode: "ask",
        enabled: true,
        builtin: false,
        slug: "analytics",
        unavailable: mcpConnected ? null : "needs_sign_in",
        sign_in: mcpConnected ? "signed_in" : "not_signed_in",
        // This row signed in through dynamic registration, so no operator
        // supplied a client for it.
        oauth_client_id: null,
        oauth_client_confidential: false,
      });
      if (mcpRecovery)
        Object.defineProperty(window, "__raiseMcpReauth", {
          value: () => {
            const handlers = eventListeners.get("mcp:reauth-required") ?? [];
            handlers.forEach((handler) =>
              callbacks.get(handler)?.({
                event: "mcp:reauth-required",
                id: handler,
                payload: { version: 1, id: "srv-expired", label: "Analytics", reason: "credential_refused" },
              }),
            );
            return handlers.length;
          },
        });
      let bundledMcpEnabled = true;
      let addedMcpServer: McpServerView | null = null;
      let nextCallback = 1;
      const transformCallback = (callback: (data: unknown) => void) => {
        const id = nextCallback++;
        callbacks.set(id, callback);
        return id;
      };

      Object.defineProperty(window, "__TAURI_EVENT_PLUGIN_INTERNALS__", {
        value: { unregisterListener() {} },
      });
      Object.defineProperty(window, "__TAURI_INTERNALS__", {
        configurable: false,
        value: Object.freeze({
          transformCallback,
          invoke(command: string, args?: Record<string, unknown>) {
            if (command === "plugin:event|listen") {
              const event = String(args?.event);
              const handler = Number(args?.handler);
              eventListeners.set(event, [...(eventListeners.get(event) ?? []), handler]);
              return Promise.resolve(handler);
            }
            if (command === "plugin:event|unlisten") {
              const event = String(args?.event);
              const id = Number(args?.eventId);
              eventListeners.set(
                event,
                (eventListeners.get(event) ?? []).filter((handler) => handler !== id),
              );
              callbacks.delete(id);
              return Promise.resolve(null);
            }
            if (updatesUpToDate && command === "update_check") {
              updateCalls.push(command);
              return Promise.resolve({
                version: 1,
                current_version: mockBootstrap.application_version,
                available_version: null,
                release_notes: null,
                is_staged: false,
              });
            }
            if (automaticUpdate && command.startsWith("update_")) {
              updateCalls.push(command);
              if (command === "update_check")
                return Promise.resolve({
                  version: 1,
                  current_version: "0.5.4",
                  available_version: "0.5.5",
                  release_notes: releaseNotes,
                  is_staged: false,
                });
              if (command === "update_install") return Promise.resolve();
              return Promise.reject(new Error("The test must not restart automatically"));
            }
            if (command === "preview_document_register" || command === "project_preview_register") {
              previewRegistrationCalls.push(command);
              return Promise.resolve("visual-preview-registration");
            }
            if (command === "app_bootstrap") {
              if (bootstrapPending) return new Promise(() => {});
              if (bootstrapReject) return Promise.reject(new Error("native runtime unreachable"));
              return Promise.resolve(mockBootstrap);
            }
            if (command === "app_health") {
              return Promise.resolve({ version: 1, status: "ready" });
            }
            if (command === "app_reset") return Promise.resolve(undefined);
            if (command === "chat_conversation_reset_active") return Promise.resolve(null);
            if (command === "delight_greeting_claim") return Promise.resolve(false);
            if (command === "delight_message_sent") return Promise.resolve(1);
            if (command === "settings_appearance_get") {
              return Promise.resolve(appearance);
            }
            if (command === "settings_appearance_set") {
              appearance = args?.appearance as typeof appearance;
              return Promise.resolve(appearance);
            }
            if (command === "credential_list") return Promise.resolve(providerCredentials);
            if (modelSettingsReady && command === "chat_model_catalog") return Promise.resolve(modelCatalog);
            if (modelSettingsReady && command === "chat_custom_model_list") return Promise.resolve(mockCustomModels);
            if (modelSettingsReady && command === "chat_custom_model_discover") {
              const model = mockCustomModels[0];
              return Promise.resolve(
                privateModelDiscovery === "unsupported"
                  ? { version: 1, models: [], existing_models: [], unavailable_reason: "unsupported" }
                  : {
                      version: 1,
                      unavailable_reason: null,
                      existing_models: [],
                      models: [
                        {
                          upstream_model_id: model.upstream_model_id,
                          display_name: model.display_name,
                          context_length: model.context_length,
                          max_output_tokens: model.max_output_tokens,
                          supports_tools: model.supports_tools,
                          supports_vision: model.supports_vision,
                          sampling: { temperature: 0.6, top_p: 0.95 },
                          supported_sampling_parameters: ["temperature", "top_p"],
                        },
                      ],
                    },
              );
            }
            if (modelSettingsReady && command === "chat_custom_model_create") {
              const input = args?.input as CustomModelCreateInput;
              const created = {
                base_url: input.base_url,
                upstream_model_id: input.upstream_model_id,
                display_name: input.display_name,
                context_length: input.context_length,
                max_output_tokens: input.max_output_tokens,
                supports_tools: input.supports_tools,
                supports_vision: input.supports_vision,
                sampling: input.sampling,
                version: 1 as const,
                id: "visual-created-model",
                model_id: "custom/visual-created-model",
                is_enabled: true,
                is_active: true,
              };
              mockCustomModels.push(created);
              return Promise.resolve(created);
            }
            if (modelSettingsReady && command === "chat_default_model_set") {
              const selected = String(args?.modelId ?? "");
              if (!modelCatalog.models.some((model) => model.id === selected)) {
                return Promise.reject({ version: 1, code: "invalid_request" });
              }
              modelCatalog = { ...modelCatalog, selected_model_id: selected };
              return Promise.resolve(modelCatalog);
            }
            if (command === "mcp_server_list")
              return Promise.resolve({
                version: 1,
                servers: [
                  ...(bundledMcpEnabled ? [bundledMcpServer()] : []),
                  ...(mcpRecovery ? [mcpServer()] : []),
                  ...(addedMcpServer ? [addedMcpServer] : []),
                ],
              });
            if (command === "chat_mcp_selection_get") return Promise.resolve({ version: 1, server_id: null });
            if (command === "mcp_server_delete") {
              if (args?.id === "tenant") bundledMcpEnabled = false;
              else if (args?.id === addedMcpServer?.id) addedMcpServer = null;
              else return Promise.reject({ version: 1, code: "not_found" });
              return Promise.resolve(null);
            }
            if (command === "mcp_server_restore_bundled") {
              bundledMcpEnabled = true;
              return Promise.resolve(bundledMcpServer());
            }
            if (command === "mcp_server_create") {
              const input = args?.input as McpServerInput;
              // Mirrors the real command: an `oauth` client is sealed on this
              // device by the create itself and reported back on the row, so
              // the edit sheet can show it rather than an empty field.
              const clientId = input.auth_type === "oauth" ? ((args?.oauthClientId as string) ?? null) : null;
              addedMcpServer = {
                version: 1,
                id: "srv-created",
                name: input.name,
                display_name: input.display_name,
                description: input.description,
                url: input.url,
                transport: "http",
                auth_type: input.auth_type,
                permission_mode: input.permission_mode,
                enabled: input.enabled,
                builtin: false,
                slug: input.name,
                sign_in: input.auth_type === "oauth" ? "not_signed_in" : null,
                unavailable: input.auth_type === "oauth" ? "needs_sign_in" : null,
                oauth_client_id: clientId,
                oauth_client_confidential: clientId != null && args?.oauthClientSecret != null,
              };
              return Promise.resolve(addedMcpServer);
            }
            if (command === "mcp_server_sign_in" && addedMcpServer && args?.id === addedMcpServer.id) {
              addedMcpServer = { ...addedMcpServer, sign_in: "signed_in", unavailable: null };
              return Promise.resolve(addedMcpServer);
            }
            if (mcpRecovery && command === "mcp_server_sign_in") {
              mcpConnected = true;
              return Promise.resolve(mcpServer());
            }
            if (mcpRecovery && command === "mcp_server_test")
              return Promise.resolve({ version: 1, id: "srv-expired", state: "connected", tools: [], message: null });
            if (command === "skill_list") return Promise.resolve({ version: 1, skills });
            if (command === "workspace_status") {
              return Promise.resolve({
                version: 1,
                tenant_host: "tenant.example",
                mcp_resource_url: "https://tenant.example/mcp",
                mcp_servers: [],
                models_available: 1,
                default_model_id: "gpt-test",
                default_provider: "openai",
                chat_ready: true,
                setup_note: "Ready",
              });
            }
            if (command === "chat_message_feedback_list") return Promise.resolve([]);
            if (command === "chat_follow_ups") return Promise.resolve({ suggestions: [] });
            if (command === "chat_workspace_bootstrap") {
              return pendingWorkspaceBootstrap ?? Promise.resolve(workspaceBootstrap);
            }
            if (command === "project_list") return Promise.resolve({ version: 1, projects });
            if (command === "project_list_directory") {
              return Promise.resolve({ version: 1, project_id: "project-visual-0", path: null, entries: [] });
            }
            if (command === "auth_cancel") {
              return Promise.resolve({ status: "unauthenticated", version: 1 });
            }
            if (command === "auth_login") {
              return Promise.reject(new Error("visual test does not open a browser"));
            }
            if (command === "diagnostics_log_renderer_error") {
              return Promise.resolve(undefined);
            }
            return Promise.reject(new Error(`visual test rejected an unknown native command: ${command}`));
          },
        }),
        writable: false,
      });
    },
    {
      automaticUpdate,
      updatesUpToDate,
      arrivalPending,
      bootstrapPending,
      bootstrapReject,
      longLabels,
      mcpRecovery,
      longLabel: LONG_LABEL,
      mockModelCatalog: visualModelCatalog,
      mockCustomModels: visualCustomModels,
      privateModelDiscovery,
      modelSettingsReady,
      providerCredentials,
      releaseNotes,
      skills,
      mockBootstrap: bootstrapWithSession(sessionStatus, longLabels),
      populatedSidebar,
      theme,
    },
  );
}
