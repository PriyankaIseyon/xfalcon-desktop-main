import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
export { projectOfficePreview, reportOfficePreview } from "./projectOfficePreview";
import type {
  AppBootstrap,
  AppearanceSettings,
  AppHealth,
  ChatConversationView,
  ChatMessageFeedback,
  ChatFollowUpsResult,
  ChatgptPlanStatusView,
  ChatModelCatalogView,
  ChatMcpSelectionView,
  ChatSubmitResult,
  ChatWorkspaceBootstrap,
  ClaudeEffortInput,
  ClaudeRuntimeStatusView,
  CustomModelCreateInput,
  CustomModelDiscoveryInput,
  CustomModelDiscoveryResult,
  CustomModelUpdateInput,
  CustomModelView,
  ConversationId,
  DatasetDetail,
  DatasetDropOutcome,
  DatasetListResult,
  DatasetPreview,
  DatasetRecipe,
  DatasetUploadRequest,
  DatasetUploadStartResult,
  DiagnosticsBundleResponse,
  DocumentExportFormat,
  DocumentExportResult,
  McpConnectionReport,
  McpReauthRequired,
  McpServerId,
  McpServerInput,
  McpServerListResult,
  McpServerView,
  PlanEnableInput,
  ProjectDirectoryListing,
  ProjectFileRestoreResult,
  FolderGrantView,
  ProjectFileRevisionDiff,
  ProjectFileRevisionList,
  ProjectFileStat,
  ProjectDocumentRead,
  ProjectId,
  ProjectListResult,
  ProjectReportSaveResult,
  ProjectSchemaSummary,
  ProjectTableRows,
  ProjectView,
  ProviderCredentialView,
  ProviderId,
  RendererErrorReport,
  ReportDocument,
  ReportId,
  ReportListResult,
  ReportRefreshResult,
  ReportThumbnail,
  ReportCaptureDocument,
  ReportScope,
  ReportView,
  SessionProjection,
  SkillErrorCode,
  SkillImportResult,
  SkillListResult,
  SkillOperationError,
  SkillPickKind,
  SkillSourceGrantView,
  SkillView,
  StagedDatasetView,
  UploadQueueV1,
  UpdateCheckResult,
  UpdateDownloadProgress,
  WorkspaceStatus,
} from "./generated/contracts";

export function appBootstrap(): Promise<AppBootstrap> {
  return invoke<AppBootstrap>("app_bootstrap");
}

export function appHealth(): Promise<AppHealth> {
  return invoke<AppHealth>("app_health");
}

export function updateCheck(): Promise<UpdateCheckResult> {
  return invoke<UpdateCheckResult>("update_check");
}

export function updateInstall(): Promise<void> {
  return invoke<void>("update_install");
}

export function updateRestart(): Promise<void> {
  return invoke<void>("update_restart");
}

export function onUpdateDownloadProgress(
  handler: (progress: UpdateDownloadProgress) => void,
): Promise<UnlistenFn> {
  return listen<UpdateDownloadProgress>("xfalcon://update-download-progress", (event) => handler(event.payload));
}

export function appReset(): Promise<void> {
  return invoke<void>("app_reset");
}

export function authStatus(): Promise<SessionProjection> {
  return invoke<SessionProjection>("auth_status");
}

export function authLogin(): Promise<SessionProjection> {
  return invoke<SessionProjection>("auth_login");
}

export function authCancel(): Promise<SessionProjection> {
  return invoke<SessionProjection>("auth_cancel");
}

export function authLogout(): Promise<SessionProjection> {
  return invoke<SessionProjection>("auth_logout");
}

export function workspaceStatus(): Promise<WorkspaceStatus> {
  return invoke<WorkspaceStatus>("workspace_status");
}

export function chatWorkspaceBootstrap(): Promise<ChatWorkspaceBootstrap> {
  return invoke<ChatWorkspaceBootstrap>("chat_workspace_bootstrap");
}

export function chatModelCatalog(conversationId?: ConversationId | null): Promise<ChatModelCatalogView> {
  return invoke<ChatModelCatalogView>("chat_model_catalog", { conversationId: conversationId ?? null });
}

export function customModelList(): Promise<CustomModelView[]> {
  return invoke<CustomModelView[]>("chat_custom_model_list");
}

export function customModelCreate(input: CustomModelCreateInput): Promise<CustomModelView> {
  return invoke<CustomModelView>("chat_custom_model_create", { input });
}

export function customModelDiscover(input: CustomModelDiscoveryInput): Promise<CustomModelDiscoveryResult> {
  return invoke<CustomModelDiscoveryResult>("chat_custom_model_discover", { input });
}

export function customModelUpdate(id: string, input: CustomModelUpdateInput): Promise<CustomModelView> {
  return invoke<CustomModelView>("chat_custom_model_update", { id, input });
}

export function customModelDelete(id: string): Promise<void> {
  return invoke<void>("chat_custom_model_delete", { id });
}

export function chatDefaultModelSet(modelId: string): Promise<ChatModelCatalogView> {
  return invoke<ChatModelCatalogView>("chat_default_model_set", { modelId });
}

export function chatConversationCreate(
  openingMessage: string | undefined,
  mcpServerId: McpServerId | null,
): Promise<ChatConversationView> {
  return invoke<ChatConversationView>("chat_conversation_create", {
    openingMessage: openingMessage ?? null,
    mcpServerId,
  });
}

export function chatMcpSelectionGet(conversationId: ConversationId): Promise<ChatMcpSelectionView> {
  return invoke<ChatMcpSelectionView>("chat_mcp_selection_get", { conversationId });
}

export function chatConversationOpen(conversationId: ConversationId): Promise<ChatConversationView> {
  return invoke<ChatConversationView>("chat_conversation_open", { conversationId });
}

/** Reads only the account-bound encrypted projection; server truth follows via open. */
export function chatConversationPreview(conversationId: ConversationId): Promise<ChatConversationView | null> {
  return invoke<ChatConversationView | null>("chat_conversation_preview", { conversationId });
}

/** Finds conversations whose message text matches the user's raw query. */
export function chatConversationSearch(query: string): Promise<ConversationId[]> {
  return invoke<ConversationId[]>("chat_conversation_search", { query });
}

export function chatConversationRename(
  conversationId: ConversationId,
  title: string,
): Promise<ChatConversationView> {
  return invoke<ChatConversationView>("chat_conversation_rename", { conversationId, title });
}

export function chatMessageSend(
  conversationId: ConversationId | null,
  prompt: string,
  attachmentIds: string[],
  mcpServerId: McpServerId | null,
): Promise<ChatSubmitResult> {
  return invoke<ChatSubmitResult>("chat_message_send", {
    conversationId,
    prompt,
    attachmentIds: attachmentIds ?? [],
    mcpServerId,
  });
}

export function chatMessageEdit(
  conversationId: ConversationId,
  messageId: string,
  text: string,
  mcpServerId: McpServerId | null,
): Promise<ChatSubmitResult> {
  return invoke<ChatSubmitResult>("chat_message_edit", {
    conversationId,
    messageId,
    text,
    mcpServerId,
  });
}

export function chatMessageRegenerate(
  conversationId: ConversationId,
  messageId: string,
  mcpServerId: McpServerId | null,
): Promise<ChatSubmitResult> {
  return invoke<ChatSubmitResult>("chat_message_regenerate", {
    conversationId,
    messageId,
    mcpServerId,
  });
}

export function chatConversationPin(
  conversationId: ConversationId,
  pinned: boolean,
): Promise<ChatWorkspaceBootstrap> {
  return invoke<ChatWorkspaceBootstrap>("chat_conversation_pin", { conversationId, pinned });
}

export function chatConversationDelete(conversationId: ConversationId): Promise<void> {
  return invoke<void>("chat_conversation_delete", { conversationId });
}

export function chatRunCancel(runId: string): Promise<void> {
  return invoke<void>("chat_run_cancel", { runId });
}

export function chatRunRetry(runId: string, mcpServerId: McpServerId | null): Promise<ChatSubmitResult> {
  return invoke<ChatSubmitResult>("chat_run_retry", { runId, mcpServerId });
}

export function chatConversationResetActive(): Promise<void> {
  return invoke<void>("chat_conversation_reset_active");
}

/**
 * Writes `contents` to a file wherever the person points the native save dialog
 * (spec: lean-report-export).
 *
 * There is no destination parameter, and that is the design: the save dialog
 * opens in Rust, so the renderer can ask for a save but cannot aim one. This is
 * the ONLY way the app hands a file to the person — an `<a download>` with a
 * blob would write with no destination step at all, which is the affordance
 * `src-tauri/src/security/tests.rs` records as refused.
 *
 * A resolved result with `saved: false` means the dialog was dismissed — not a
 * failure, and callers must not report it as one.
 */
export function documentExportSave(
  suggestedName: string,
  contents: string,
  format: DocumentExportFormat,
): Promise<DocumentExportResult> {
  return invoke<DocumentExportResult>("document_export_save", {
    suggestedName,
    contents,
    format,
  });
}

/**
 * Writes a PDF of the currently mounted print sheet wherever the person
 * points the native save dialog. The renderer sends only a suggested name —
 * never a path. A dismissed dialog resolves with `saved: false`.
 */
export function documentExportPdf(suggestedName: string): Promise<DocumentExportResult> {
  return invoke<DocumentExportResult>("document_export_pdf", { suggestedName });
}

/** Opens the native folder picker. Resolves to a grant for the chosen folder,
 * or `null` if the person cancelled -- which is not an error. The renderer gets
 * the grant id to act with and `display_path` to show; it never holds the path. */
export function projectPickFolder(): Promise<FolderGrantView | null> {
  return invoke<FolderGrantView | null>("project_pick_folder");
}

/** Trusts the folder a grant from `projectPickFolder` names. */
export function projectCreate(grantId: string, displayName?: string): Promise<ProjectView> {
  return invoke<ProjectView>("project_create", { grantId, displayName: displayName ?? null });
}

/** Lists trusted projects on the folder a grant from `projectPickFolder` names. */
export function projectFindByFolder(grantId: string): Promise<ProjectListResult> {
  return invoke<ProjectListResult>("project_find_by_folder", { grantId });
}

export function projectList(): Promise<ProjectListResult> {
  return invoke<ProjectListResult>("project_list");
}

export function projectActivate(projectId: ProjectId): Promise<ProjectView> {
  return invoke<ProjectView>("project_activate", { projectId });
}

export function projectDeactivate(): Promise<void> {
  return invoke<void>("project_deactivate");
}

export function projectRevoke(projectId: ProjectId): Promise<void> {
  return invoke<void>("project_revoke", { projectId });
}

/** Lists one directory's immediate children inside a trusted project root.
 * `path` of `null` lists the root. Never recursive -- one call is one level, so
 * a tree expands only where the user actually looked. */
export function projectListDirectory(projectId: ProjectId, path: string | null): Promise<ProjectDirectoryListing> {
  return invoke<ProjectDirectoryListing>("project_list_directory", { projectId, path });
}

/** A project file's size, modified time, and kind, before any byte is read
 * past a prefix sniff. Sensitive paths are classified from the path alone. */
export function projectFileStat(projectId: ProjectId, path: string): Promise<ProjectFileStat> {
  return invoke<ProjectFileStat>("project_file_stat", { projectId, path });
}

/** Saves the PDF of a finished Office preview wherever the person picks in the
 * native save dialog. The backend holds the bytes; only the preview's id goes
 * here, never a path. A dismissed dialog resolves with `saved: false`. */
export function projectOfficeSavePdf(projectId: ProjectId, path: string, renderId: string): Promise<DocumentExportResult> {
  return invoke<DocumentExportResult>("project_office_save_pdf", { projectId, path, renderId });
}

/** Saves a copy of a project Office file wherever the person picks. The
 * project's own file is only read. */
export function projectOfficeSaveOriginal(projectId: ProjectId, path: string): Promise<DocumentExportResult> {
  return invoke<DocumentExportResult>("project_office_save_original", { projectId, path });
}

/** Reads bounded, text-only content from a project Office file. This endpoint
 * is read-only and leaves the original binary bytes untouched. */
export function projectDocumentRead(projectId: ProjectId, path: string): Promise<ProjectDocumentRead> {
  return invoke<ProjectDocumentRead>("project_document_read", { projectId, path });
}

/** The largest range one {@link projectFileReadBytes} call returns
 * (`MAX_READ_RANGE_BYTES` in `src-tauri/src/projects/viewer.rs`). */
export const PROJECT_READ_RANGE_MAX_BYTES = 8 * 1024 * 1024;

/**
 * Up to `length` raw bytes of a project file from `offset`, as an
 * `ArrayBuffer` — never a JSON string. Fewer only at the end of the file. The
 * caller decodes and aligns: a range may start or end inside a line or a
 * character.
 */
export async function projectFileReadBytes(
  projectId: ProjectId,
  path: string,
  offset: number,
  length: number,
): Promise<Uint8Array> {
  const buffer = await invoke<ArrayBuffer>("project_file_read_bytes", { projectId, path, offset, length });
  return new Uint8Array(buffer);
}

/** Registers an HTML or SVG project file for preview, read by the backend so
 * the document never crosses IPC. Returns the preview document id. */
export function projectPreviewRegister(projectId: ProjectId, path: string, kind: PreviewDocumentKind): Promise<string> {
  return invoke<string>("project_preview_register", { projectId, path, kind });
}

/** One window of a delimited file's rows, parsed by the backend. `delimiter`
 * is `","` or `"\t"`; `startRow` counts data rows from zero after the header. */
export function projectTableRows(
  projectId: ProjectId,
  path: string,
  delimiter: "," | "\t",
  startRow: number,
  limit: number,
): Promise<ProjectTableRows> {
  return invoke<ProjectTableRows>("project_table_rows", { projectId, path, delimiter, startRow, limit });
}

/** Extracts schemas for tabular/structured files (CSV, JSON, SQLite) in the trusted project root. */
export function projectSchemaSummary(projectId: ProjectId): Promise<ProjectSchemaSummary> {
  return invoke<ProjectSchemaSummary>("project_schema_summary", { projectId });
}

/** The provider keys registered for this account, as redacted projections --
 * no secret is ever returned, by design. */
export function credentialList(): Promise<ProviderCredentialView[]> {
  return invoke<ProviderCredentialView[]>("credential_list");
}

export function credentialCreate(
  provider: ProviderId,
  label: string,
  secret: string,
  isDefault: boolean,
): Promise<ProviderCredentialView> {
  return invoke<ProviderCredentialView>("credential_create", { provider, label, secret, isDefault });
}

/** Re-seals an existing key to this device, keeping its row, label, and default
 * flag. This is what a `needs_reentry` status asks for: adding the key again
 * under the same label collides with the workspace's uniqueness rule, and under
 * a new label the unusable row would survive holding the default flag. */
export function credentialReplace(id: string, secret: string): Promise<ProviderCredentialView> {
  return invoke<ProviderCredentialView>("credential_replace", { id, secret });
}

/** Deletes a stored key. The row is shared, so every signed-in xFalcon client
 * loses it, not only this desktop. */
export function credentialDelete(id: string): Promise<void> {
  return invoke<void>("credential_delete", { id });
}

/** The Claude runtime on this device: whether it is enabled, and what the
 * installed `claude` program reports. Reads no credential. */
export function claudeRuntimeStatus(): Promise<ClaudeRuntimeStatusView> {
  return invoke<ClaudeRuntimeStatusView>("claude_runtime_status");
}

/** Turns the Claude runtime on or off for this device. The caller shows the
 * consent notice before it asks for `enabled: true`. */
export function claudeRuntimeSetEnabled(input: PlanEnableInput): Promise<ClaudeRuntimeStatusView> {
  return invoke<ClaudeRuntimeStatusView>("claude_runtime_set_enabled", { input });
}

/** Sets the reasoning level a new Claude chat starts with. A chat that has
 * already started keeps the level it started with. */
export function claudeRuntimeSetEffort(input: ClaudeEffortInput): Promise<ClaudeRuntimeStatusView> {
  return invoke<ClaudeRuntimeStatusView>("claude_runtime_set_effort", { input });
}

/** The ChatGPT plan on this device. Reads no token. */
export function chatgptPlanStatus(): Promise<ChatgptPlanStatusView> {
  return invoke<ChatgptPlanStatusView>("chatgpt_plan_status");
}

/** Opens the system browser and resolves only when the sign-in ends, which can
 * take up to five minutes. {@link chatgptPlanSignInCancel} ends it early. */
export function chatgptPlanSignIn(): Promise<ChatgptPlanStatusView> {
  return invoke<ChatgptPlanStatusView>("chatgpt_plan_sign_in");
}

export function chatgptPlanSignInCancel(): Promise<void> {
  return invoke<void>("chatgpt_plan_sign_in_cancel");
}

export function chatgptPlanSignOut(): Promise<ChatgptPlanStatusView> {
  return invoke<ChatgptPlanStatusView>("chatgpt_plan_sign_out");
}

/** Records that the first-sign-in notice was seen, so it is not shown again. */
export function chatgptPlanWelcomeAck(): Promise<ChatgptPlanStatusView> {
  return invoke<ChatgptPlanStatusView>("chatgpt_plan_welcome_ack");
}

/** The bundled tenant server followed by every MCP server registered on this
 * account, each already classified against what this build and this device can
 * actually connect to (`McpServerView.unavailable`). */
export function mcpServerList(): Promise<McpServerListResult> {
  return invoke<McpServerListResult>("mcp_server_list");
}

/** The operator-issued OAuth client an `oauth` server is registered with.
 *
 * Sealed into this device's vault beside the server's other material and never
 * sent to the shared registry. `clientId` of `null` means "this device holds no
 * client for this server", which is also how a stored one is removed: the edit
 * sheet is seeded with what is stored, so what it submits is the state to keep.
 *
 * `clientSecret` of `null` keeps whatever is already sealed for `clientId` --
 * the sheet cannot seed a confidential value, so blank has to mean "unchanged".
 * It is dropped only when `clientId` itself changes, because a value issued for
 * one client does not authenticate another. */
export interface McpOAuthClient {
  clientId: string | null;
  clientSecret: string | null;
}

/** Registers a server definition in the shared account registry and seals its
 * `oauth` client into this device's vault. The client never crosses the registry
 * wire, so it authenticates on this device only.
 *
 * The command also accepts a static credential for a `header` or `bearer` row.
 * Nothing here sends one: this app registers browser sign-in only, so the
 * argument goes over as `null` and the backend seals nothing. Sent explicitly
 * rather than omitted, so the payload states what it means instead of relying on
 * a missing key deserializing into `None`. */
export function mcpServerCreate(input: McpServerInput, oauth: McpOAuthClient): Promise<McpServerView> {
  return invoke<McpServerView>("mcp_server_create", {
    input,
    credential: null,
    oauthClientId: oauth.clientId,
    oauthClientSecret: oauth.clientSecret,
  });
}

/** Updates a registered server. See [`McpOAuthClient`] for how `oauth` is
 * applied, and [`mcpServerCreate`] for the credential this app never sends. */
export function mcpServerUpdate(
  id: McpServerId,
  input: McpServerInput,
  oauth: McpOAuthClient,
): Promise<McpServerView> {
  return invoke<McpServerView>("mcp_server_update", {
    id,
    input,
    credential: null,
    oauthClientId: oauth.clientId,
    oauthClientSecret: oauth.clientSecret,
  });
}

/** Removes a registered server from the account, or disables the bundled connection on this device. */
export function mcpServerDelete(id: McpServerId): Promise<void> {
  return invoke<void>("mcp_server_delete", { id });
}

/** Restores this account's bundled workspace MCP connection on this device. */
export function mcpServerRestoreBundled(): Promise<McpServerView> {
  return invoke<McpServerView>("mcp_server_restore_bundled");
}

/** Signs in to one registered OAuth server. Opens the system browser, waits for
 * the redirect, and seals the resulting grant to this device -- it is never sent
 * to the shared registry, so signing in here signs in on this device only.
 *
 * Optional operator-issued `clientId` / `clientSecret` take precedence over
 * automatic client selection, and are sealed to this device before the browser
 * opens -- so a flow the user then declines does not discard them. Pass `null`
 * for both to use whatever this device already holds, then a packaged client,
 * then dynamic registration; the `client_registration_unavailable` failure is
 * what asks for manual details. */
export function mcpServerSignIn(
  id: McpServerId,
  clientId: string | null,
  clientSecret: string | null,
): Promise<McpServerView> {
  return invoke<McpServerView>("mcp_server_sign_in", { id, clientId, clientSecret });
}

/**
 * Subscribes to "an MCP server refused its saved sign-in during a run".
 *
 * Emitted by the backend the moment a refusal is classified, so the user is
 * asked while the run is still in front of them rather than discovering later
 * that a server's tools were quietly missing. One event per server per run —
 * the deduplication happens where the count is known, not here.
 */
export async function onMcpReauthRequired(
  handler: (prompt: McpReauthRequired) => void,
): Promise<UnlistenFn> {
  return listen<McpReauthRequired>("mcp:reauth-required", (event) => handler(event.payload));
}

/** Signs out of one registered OAuth server: revoked with its authorization
 * server where that is supported, and forgotten on this device either way. The
 * shared server definition is untouched. */
export function mcpServerSignOut(id: McpServerId): Promise<McpServerView> {
  return invoke<McpServerView>("mcp_server_sign_out", { id });
}

/** Dials one server now and reports what this device observed. Stores nothing:
 * a laptop behind a captive portal must not mark a healthy server broken for
 * the whole tenant. */
export function mcpServerTest(id: McpServerId): Promise<McpConnectionReport> {
  return invoke<McpConnectionReport>("mcp_server_test", { id });
}

/**
 * Writes `content` over one file inside a trusted project, and resolves once
 * the bytes are on disk.
 *
 * There is no review step: these are local files the operator chose to trust,
 * and a person saving their own edit has already reviewed it. The backend still
 * plans, journals and audits the change, and re-validates containment before
 * writing (`ApprovalsService::save`).
 */
export function projectSaveFile(projectId: ProjectId, path: string, contentBase64: string): Promise<void> {
  return invoke<void>("project_save_file", { projectId, path, contentBase64 });
}

export function projectFileRevisions(projectId: ProjectId, relativePath: string): Promise<ProjectFileRevisionList> {
  return invoke<ProjectFileRevisionList>("project_file_revisions", { projectId, relativePath });
}

/** What changed in a file from one of its revisions to `againstBackupId` -- a
 * newer revision of the same file -- or, when that is `null`, to the file now.
 * The backend refuses, rather than cuts, a diff past its size limits. */
export function projectFileRevisionDiff(
  projectId: ProjectId,
  relativePath: string,
  backupId: string,
  againstBackupId: string | null,
): Promise<ProjectFileRevisionDiff> {
  return invoke<ProjectFileRevisionDiff>("project_file_revision_diff", {
    projectId,
    relativePath,
    backupId,
    againstBackupId,
  });
}

export function projectFileRestore(backupId: string): Promise<ProjectFileRestoreResult> {
  return invoke<ProjectFileRestoreResult>("project_file_restore", { backupId });
}

/** Approves or declines one tool call an agent run is paused on. Approving
 * mints a token scoped to that exact invocation and tool inside the run's own
 * `ToolLoop`; it grants nothing else. */
export function chatToolApprovalResolve(
  runId: string,
  invocationId: string,
  approve: boolean,
): Promise<void> {
  return invoke<void>("chat_tool_approval_resolve", { runId, invocationId, approve });
}

/** This device's appearance preference. Never fails: an unreadable or absent
 * settings file resolves to the defaults (dark, small). */
export function appearanceGet(): Promise<AppearanceSettings> {
  return invoke<AppearanceSettings>("settings_appearance_get");
}

/** Persists the appearance preference and echoes what was stored. Rejects
 * with a `SettingsOperationError` if the file could not be written -- the UI
 * has already applied the choice, so the caller must say so rather than
 * report success. */
export function appearanceSet(appearance: AppearanceSettings): Promise<AppearanceSettings> {
  return invoke<AppearanceSettings>("settings_appearance_set", { appearance });
}

/** Claims the day's greeting. Resolves `true` for the first claim of the local
 * calendar day `day` (`YYYY-MM-DD`) on this device, `false` for every later one.
 * Rejects when the claim could not be recorded; callers carry on without it. */
export function delightGreetingClaim(day: string): Promise<boolean> {
  return invoke<boolean>("delight_greeting_claim", { day });
}

/** Counts one message sent from this device and resolves the new total. */
export function delightMessageSent(): Promise<number> {
  return invoke<number>("delight_message_sent");
}

/** Every skill the signed-in account imported, metadata only. A skill's instructions body is never
 * returned here -- the model fetches it by name during a run. */
export function skillList(): Promise<SkillListResult> {
  return invoke<SkillListResult>("skill_list");
}

/** Opens the native picker Settings asked for -- skill files and archives, or a
 * folder to search for skills under. Resolves to a grant over what was chosen,
 * or `null` if the person cancelled. */
export function skillPick(kind: SkillPickKind): Promise<SkillSourceGrantView | null> {
  return invoke<SkillSourceGrantView | null>("skill_pick", { kind });
}

/** What a drag over the window is doing, or a drop that was granted or refused. */
export type SkillSourcesEvent =
  | { kind: "over" }
  | { kind: "leave" }
  | { kind: "dropped"; grant: SkillSourceGrantView }
  | { kind: "rejected"; code: SkillErrorCode };

/**
 * Reports skills dragged onto the window. A drop is read and granted in Rust, so
 * the renderer gets a grant to import with and never the dropped paths.
 *
 * The events are webview-wide, not element-scoped, so only listen while an
 * import surface is actually open; a drop anywhere on the window while it is
 * open counts as a drop on it.
 */
export async function onSkillSources(handler: (event: SkillSourcesEvent) => void): Promise<UnlistenFn> {
  const unlisteners: UnlistenFn[] = [];
  try {
    unlisteners.push(
      await listen<"over" | "leave">("xfalcon://skill-drag", (event) =>
        handler(event.payload === "over" ? { kind: "over" } : { kind: "leave" }),
      ),
    );
    unlisteners.push(
      await listen<SkillSourceGrantView>("xfalcon://skill-sources-dropped", (event) =>
        handler({ kind: "dropped", grant: event.payload }),
      ),
    );
    unlisteners.push(
      await listen<SkillOperationError>("xfalcon://skill-sources-rejected", (event) =>
        handler({ kind: "rejected", code: event.payload.code }),
      ),
    );
  } catch (cause: unknown) {
    // Do not leave the listeners already registered behind when a later one fails.
    unlisteners.forEach((stop) => stop());
    throw cause;
  }
  return () => unlisteners.forEach((stop) => stop());
}

/** Imports what a grant from `skillPick` or `onSkillSources` names -- each a skill
 * file, a `.skill` package or `.zip` to read, or a folder to search. A skill's
 * bundled files are stored with it. Reports per source: a malformed file the user
 * named comes back in `failed` with its reason while its valid siblings still
 * import. */
export function skillImport(grantId: string): Promise<SkillImportResult> {
  return invoke<SkillImportResult>("skill_import", { grantId });
}

/** Enables or disables one skill by name. A disabled skill stays on disk but
 * reaches no run -- neither advertised nor fetchable. */
export function skillSetEnabled(name: string, enabled: boolean): Promise<SkillView> {
  return invoke<SkillView>("skill_set_enabled", { name, enabled });
}

/** Removes one skill's stored directory and forgets its enabled state, so a
 * later import of the same name starts enabled. */
export function skillDelete(name: string): Promise<void> {
  return invoke<void>("skill_delete", { name });
}

/** A previewable document kind the `xf-preview` scheme can serve
 * (spec: preview-protocol). */
export type PreviewDocumentKind = "html" | "svg";

/** Registers one previewable document into the bounded in-memory preview
 * store and resolves to its content-addressed id — identical content
 * re-registers to the same id, so re-renders are free. */
export function previewDocumentRegister(kind: PreviewDocumentKind, content: string): Promise<string> {
  return invoke<string>("preview_document_register", { kind, content });
}

/** The platform-correct `xf-preview` URL for a registered document id
 * (`http://xf-preview.localhost/<id>` on Windows, `xf-preview://localhost/<id>`
 * elsewhere — wry's split, resolved by the runtime). */
export function previewDocumentSrc(documentId: string): string {
  return convertFileSrc(documentId, "xf-preview");
}

/**
 * Saves one HTML document of a project conversation's assistant message into
 * that project's folder, as `reports/<name>.html`, instead of the DA store.
 *
 * Identity only: the document's bytes are re-read from the encrypted chat
 * projection, so this call cannot store markup the renderer supplies. It takes
 * no project id or path: the backend takes the project from the conversation's
 * own binding and derives the file name, so the renderer cannot choose where the
 * file lands. `not_in_project` means the conversation has no binding — nothing
 * is saved in its place.
 */
export function reportSaveToProject(
  conversationId: ConversationId,
  messageId: string,
  htmlIndex: number,
  title: string,
  allowDuplicate: boolean,
): Promise<ProjectReportSaveResult> {
  return invoke<ProjectReportSaveResult>("report_save_to_project", {
    conversationId,
    messageId,
    htmlIndex,
    title,
    allowDuplicate,
  });
}

/** Every dashboard report the account can see, in DA's order (pinned first, then
 * most recently updated). The order is DA's, not a local rule: the renderer
 * sorts with `sortReports` in `reportsStore.ts`. Metadata only -- a report's
 * document is never returned here. */
export function reportList(): Promise<ReportListResult> {
  return invoke<ReportListResult>("report_list");
}

/** Resolves a report page to a fresh preview-document id for its frame. `page`
 * is a page name from the report's `pages`, or `null` for the first page. The
 * id is in-memory and evictable, so the viewer calls this on every mount rather
 * than remembering one -- a remembered id is gone after a restart. */
export function reportOpen(reportId: ReportId, page: string | null = null): Promise<ReportDocument> {
  return invoke<ReportDocument>("report_open", { reportId, page });
}

/** A report's stored thumbnail image, as base64 PNG or WebP. Rejects with
 * `not_found` when DA holds none, and the card then draws the report itself.
 * Ask only for a report whose `thumbnail_updated_at_ms` is set. */
export function reportThumbnail(reportId: ReportId): Promise<ReportThumbnail> {
  return invoke<ReportThumbnail>("report_thumbnail", { reportId });
}

/** A dashboard's first page with DA's thumbnail capture script, for a frame
 * that is never shown, and the nonce that script answers to. Owner use only:
 * the image it makes is stored with `reportThumbnailSave`. */
export function reportThumbnailCaptureOpen(reportId: ReportId): Promise<ReportCaptureDocument> {
  return invoke<ReportCaptureDocument>("report_thumbnail_capture_open", { reportId });
}

/** Stores a captured PNG (base64) as the thumbnail of a report the reader owns,
 * replacing any it had, and returns the report with its new image version. The
 * backend refuses anything but a PNG within 200 KiB as `invalid_thumbnail`. */
export function reportThumbnailSave(reportId: ReportId, imageBase64: string): Promise<ReportView> {
  return invoke<ReportView>("report_thumbnail_save", { reportId, imageBase64 });
}

/** Re-runs every stored dataset of a report in DA. No SQL or rows leave the
 * device: DA runs the stored queries under the caller's grants. A dataset that
 * fails arrives as an outcome with `ok: false`, not as a rejection, and the
 * report keeps its last good data for that dataset. */
export function reportRefresh(reportId: ReportId): Promise<ReportRefreshResult> {
  return invoke<ReportRefreshResult>("report_refresh", { reportId });
}

export function reportRename(
  reportId: ReportId,
  title: string,
  description: string | null,
): Promise<ReportView> {
  return invoke<ReportView>("report_rename", { reportId, title, description });
}

/** Pins or unpins for the calling reader only. Takes the intent. A pin changes
 * nothing for anyone else's list. */
export function reportSetPinned(reportId: ReportId, pinned: boolean): Promise<ReportView> {
  return invoke<ReportView>("report_set_pinned", { reportId, pinned });
}

export function reportDelete(reportId: ReportId): Promise<void> {
  return invoke<void>("report_delete", { reportId });
}

/** Shares a report with the organization (`global`) or makes it private again
 * (`user`). Takes the intent. DA allows it to the owner only; the caller offers
 * it on `can_manage` and DA still refuses a stale answer with `forbidden`. */
export function reportSetScope(reportId: ReportId, scope: ReportScope): Promise<ReportView> {
  return invoke<ReportView>("report_set_scope", { reportId, scope });
}

/** Hides a report from the calling reader's own list, or shows it again. Like a
 * pin it changes no one else's list, and a hidden report still opens by id. */
export function reportSetHidden(reportId: ReportId, hidden: boolean): Promise<ReportView> {
  return invoke<ReportView>("report_set_hidden", { reportId, hidden });
}

/**
 * Writes a report to a file the reader chooses: a dashboard's assembled page as
 * one `.html` file, a workbook or deck as the `.xlsx` or `.pptx` DA built.
 *
 * The save dialog opens in the backend, so this takes no path and no bytes and
 * nothing the renderer sends can aim the write. A dismissed dialog resolves as
 * `saved: false`; it is not a failure.
 */
export function reportExport(reportId: ReportId, page: string | null = null): Promise<DocumentExportResult> {
  return invoke<DocumentExportResult>("report_export", { reportId, page });
}

/**
 * Copies a library report's assembled page into `reports/` of a project the
 * reader picked. The project is named by id and resolved to a trusted root in
 * the backend; the page's bytes never cross the boundary. The copy is a snapshot
 * as of the report's last refresh and keeps no link to the report.
 */
export function reportSaveLibraryToProject(
  reportId: ReportId,
  page: string | null,
  projectId: string,
  title: string,
  allowDuplicate: boolean,
): Promise<ProjectReportSaveResult> {
  return invoke<ProjectReportSaveResult>("report_save_library_to_project", {
    reportId,
    page,
    projectId,
    title,
    allowDuplicate,
  });
}

export function diagnosticsLogRendererError(report: RendererErrorReport): Promise<void> {
  return invoke<void>("diagnostics_log_renderer_error", { report });
}

export function diagnosticsBundleGenerate(): Promise<DiagnosticsBundleResponse> {
  return invoke<DiagnosticsBundleResponse>("diagnostics_bundle_generate");
}

/** Opens the native file picker in Rust and queues the chosen chat attachment.
 * The webview never receives a filesystem path, and the transfer starts as soon
 * as the picker confirms. Datasets use `datasetPick`, which stages the file for
 * a confirmation step first. */
/** Picks a chat attachment. With `conversationId` `null` -- nothing sent yet --
 * the picker opens first, and a chosen file reserves an unopened conversation
 * that the queued upload names. A cancelled picker creates nothing. */
export function uploadChoose(conversationId: string | null, mcpServerId: McpServerId | null): Promise<UploadQueueV1> {
  return invoke<UploadQueueV1>("upload_choose", { conversationId, mcpServerId });
}

/** Clipboard bytes have no filesystem authority. Native fallback reads OS file references. */
export function uploadPaste(
  conversationId: string | null,
  name: string | null,
  dataBase64: string | null,
  mcpServerId: McpServerId | null,
): Promise<UploadQueueV1> {
  return invoke<UploadQueueV1>("upload_paste", { conversationId, name, dataBase64, mcpServerId });
}

export function uploadCancel(uploadId: string): Promise<UploadQueueV1> {
  return invoke<UploadQueueV1>("upload_cancel", { uploadId });
}

export function uploadRetry(uploadId: string): Promise<UploadQueueV1> {
  return invoke<UploadQueueV1>("upload_retry", { uploadId });
}

export function uploadQueue(): Promise<UploadQueueV1> {
  return invoke<UploadQueueV1>("upload_queue");
}

export function onUploadQueue(handler: (queue: UploadQueueV1) => void): Promise<UnlistenFn> {
  return listen<UploadQueueV1>("xfalcon://upload-queue", (event) => handler(event.payload));
}

/** Opens the native picker and stages the chosen file. Staging, not uploading:
 * the person previews it and confirms before any byte moves. The webview
 * receives an opaque id, never a path. */
export function datasetPick(): Promise<StagedDatasetView> {
  return invoke<StagedDatasetView>("dataset_pick");
}

/** The staged file's first rows, for a worksheet and header row, after the
 * recipe. The recipe is applied in Rust — the same function that produces the
 * uploaded file — so this preview is the result rather than a guess at it. */
export function datasetPreview(options: {
  stagedId: string;
  worksheetIndex?: number | null;
  headerRowIndex?: number | null;
  recipe?: DatasetRecipe | null;
}): Promise<DatasetPreview> {
  return invoke<DatasetPreview>("dataset_preview", {
    stagedId: options.stagedId,
    worksheetIndex: options.worksheetIndex ?? null,
    headerRowIndex: options.headerRowIndex ?? null,
    recipe: options.recipe ?? null,
  });
}

/** Releases a staged file the person did not upload. */
export function datasetStageDiscard(stagedId: string): Promise<void> {
  return invoke<void>("dataset_stage_discard", { stagedId });
}

/** Uploads a staged file, applying its recipe first when it has one. */
export function datasetUploadStart(request: DatasetUploadRequest): Promise<DatasetUploadStartResult> {
  return invoke<DatasetUploadStartResult>("dataset_upload_start", { request });
}

export function datasetList(): Promise<DatasetListResult> {
  return invoke<DatasetListResult>("dataset_list");
}

export function datasetDetail(datasetId: string): Promise<DatasetDetail> {
  return invoke<DatasetDetail>("dataset_detail", { datasetId });
}

export function datasetDelete(datasetId: string): Promise<void> {
  return invoke<void>("dataset_delete", { datasetId });
}

/** Saves a dataset file where the person chooses. The presigned URL is fetched
 * in Rust and never reaches the webview, and this takes no destination — the
 * backend opens the save dialog itself. */
export function datasetDownload(datasetId: string, fileId: string): Promise<DocumentExportResult> {
  return invoke<DocumentExportResult>("dataset_download", { datasetId, fileId });
}

/** Files dropped on the window, already staged or already refused. The drop is
 * handled in Rust, so no path crosses this boundary. */
export function onDatasetDropped(
  handler: (outcome: DatasetDropOutcome) => void,
): Promise<UnlistenFn> {
  return listen<DatasetDropOutcome>("xfalcon://dataset-dropped", (event) => handler(event.payload));
}

export function chatMessageFeedbackList(conversationId: string): Promise<ChatMessageFeedback[]> {
  return invoke<ChatMessageFeedback[]>("chat_message_feedback_list", { conversationId });
}

export function chatMessageFeedbackSet(conversationId: string, messageId: string, rating: 1 | -1 | null): Promise<ChatMessageFeedback | null> {
  return invoke<ChatMessageFeedback | null>("chat_message_feedback_set", { conversationId, messageId, rating });
}

export function chatFollowUps(conversationId: string, assistantContent: string, userContent: string | null): Promise<ChatFollowUpsResult> {
  return invoke<ChatFollowUpsResult>("chat_follow_ups", { conversationId, assistantContent, userContent });
}
