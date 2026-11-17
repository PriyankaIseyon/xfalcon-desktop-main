export type ContractVersion = 1;

export type AppErrorCode = "auth_required";

export type AppError = { version: 1, code: AppErrorCode, message: string, };

export type RunId = string;

export type ConversationId = string;

export type RunState = "draft" | "queued" | "running" | "awaiting_approval" | "completed" | "failed" | "cancelled" | "interrupted" | "recovering";

export type RunEvent = { "type": "state_changed", version: 1, run_id: RunId, state: RunState, };

export type RuntimeEvent = { "type": "persistence_recovered", version: 1, run_id: RunId, replayed_events: number, } | { "type": "audit_queue_capacity_reached", version: 1, maximum_events: number, };

export type Platform = "windows" | "macos" | "linux" | "unknown";

export type AppBootstrap = { version: 1, application_version: string, platform: Platform, session: SessionProjection, };

export type AppHealthStatus = "ready" | "degraded";

export type AppHealth = { version: 1, status: AppHealthStatus, };

export type LocalDataResource = "active_work" | "session_vault" | "journal" | "local_database" | "local_projections" | "project_data" | "upload_temporary" | "artifact_cache" | "export" | "decrypted_working_files";

export type CleanupErrorCode = "busy" | "no_active_account" | "vault_unavailable" | "storage_unavailable" | "cleanup_incomplete";

export type CleanupRetryPolicy = "user_initiated" | "after_restart" | "do_not_retry";

export type CleanupNextAction = "sign_in" | "retry_clear_local_data" | "restart_and_retry" | "contact_support";

export type CleanupFailure = { resource: LocalDataResource, code: CleanupErrorCode, retryable: boolean, retry_policy: CleanupRetryPolicy, next_action: CleanupNextAction, };

export type LocalDataClearStatus = "cleared" | "partial" | "failed";

export type LocalDataClearResult = { version: 1, correlation_id: string, status: LocalDataClearStatus, cleared: Array<LocalDataResource>, failures: Array<CleanupFailure>, };

export type IdentityProjection = { version: 1, user_id: string, tenant_id: string, display_name: string, email: string | null, };

export type SessionProjection = { "status": "unauthenticated", version: 1, } | { "status": "authenticating", version: 1, } | { "status": "authenticated", version: 1, identity: IdentityProjection, expires_at_ms: number, };

export type AuthLoginErrorCode = "already_authenticating" | "not_configured" | "browser_unavailable" | "discovery_failed" | "issuer_mismatch" | "discovery_metadata_invalid" | "loopback_unavailable" | "callback_invalid" | "callback_timed_out" | "redirect_not_registered" | "token_exchange_failed" | "identity_failed" | "device_clock_skew" | "cancelled" | "account_bind_failed";

export type AuthLoginError = { version: 1, code: AuthLoginErrorCode, };

export type ProviderId = "openai" | "anthropic" | "google" | "openrouter" | "custom_openai" | "claude_code" | "chatgpt_plan";

export type CredentialId = string;

export type CredentialStatus = "ready" | "disabled" | "needs_reentry" | "verification_failed";

export type ProviderCredentialView = { version: 1, id: CredentialId, provider: ProviderId, display_name: string, status: CredentialStatus, is_default: boolean, };

export type CredentialErrorCode = "invalid_request" | "invalid_credential" | "insufficient_permissions" | "provider_unavailable" | "rate_limited" | "storage_unavailable" | "service_unavailable" | "conflict" | "reconciliation_required" | "unauthorized";

export type CredentialOperationError = { version: 1, code: CredentialErrorCode,
/**
 * Present only when an ambiguous mutation was followed by a successful
 * redacted list reconciliation. `None` means reconciliation itself failed.
 */
credentials: Array<ProviderCredentialView> | null, };

export type ModelId = string;

export type ModelCapabilitiesView = { tools: boolean, image_input: boolean, reasoning_summary: boolean, };

export type ModelView = { version: 1, id: ModelId, provider: ProviderId, display_name: string, capabilities: ModelCapabilitiesView, };

export type CustomModelView = { version: 1, id: string, model_id: string, display_name: string, base_url: string, upstream_model_id: string, context_length: number, max_output_tokens: number | null, supports_tools: boolean, supports_vision: boolean, sampling: CustomModelSampling, is_enabled: boolean, is_active: boolean, };

export type CustomModelSampling = { temperature: number | null, top_p: number | null, };

export type CustomModelCreateInput = { base_url: string, secret: string, upstream_model_id: string, display_name: string, context_length: number, max_output_tokens: number, supports_tools: boolean, supports_vision: boolean, sampling: CustomModelSampling, };

export type CustomModelUpdateInput = { base_url?: string, secret?: string, is_enabled?: boolean, is_active?: boolean, upstream_model_id?: string, display_name?: string, context_length?: number, max_output_tokens?: number, supports_tools?: boolean, supports_vision?: boolean, sampling?: CustomModelSampling, };

export type CustomModelDiscoveryInput = { base_url: string, secret: string, };

export type CustomModelDiscoveryResult = { version: 1, models: Array<DiscoveredCustomModel>, existing_models: Array<ExistingCustomModel>, unavailable_reason: CustomModelDiscoveryUnavailableReason | null, };

export type CustomModelDiscoveryUnavailableReason = "unsupported" | "unreachable" | "invalid_response" | "empty";

export type DiscoveredCustomModel = { upstream_model_id: string, display_name: string, context_length: number | null, max_output_tokens: number | null, supports_tools: boolean | null, supports_vision: boolean | null, sampling: CustomModelSampling, supported_sampling_parameters: Array<string> | null, };

export type ExistingCustomModel = { upstream_model_id: string, display_name: string, };

export type ClaudeRuntimeState = "disabled" | "not_installed" | "outdated" | "signed_out" | "ready" | "check_failed";

export type ClaudeEffort = "low" | "medium" | "high" | "xhigh" | "max";

export type ClaudeRuntimeStatusView = { version: 1, enabled: boolean, state: ClaudeRuntimeState, installed_version: string | null, required_version: string, plan: string | null, sign_in_method: string | null,
/**
 * The command the user runs in their own terminal to sign in.
 */
sign_in_command: string,
/**
 * The level a new chat starts with. A chat keeps the level it started with.
 */
effort: ClaudeEffort, };

export type PlanEnableInput = { enabled: boolean, };

export type ClaudeEffortInput = { effort: ClaudeEffort, };

export type ChatgptPlanState = "signed_out" | "signing_in" | "ready" | "plan_usage_disabled" | "needs_sign_in" | "check_failed";

export type ChatgptPlanStatusView = { version: 1, state: ChatgptPlanState,
/**
 * True until the user dismisses OpenAI's first-sign-in notice.
 */
welcome_pending: boolean,
/**
 * The ChatGPT account the user signed in with, shown to that user only.
 */
account_label: string | null, model_count: number,
/**
 * OpenAI's page where the user manages plan usage.
 */
usage_url: string,
/**
 * A short, safe message for the last failure, when there is one.
 */
error: string | null, };

export const MODEL_CONTEXT_RESERVED_TOKENS = 16384;

export const MODEL_MAX_OUTPUT_TOKENS_PER_TURN = 128000;

export type ReasoningSummary = string;

export type InferenceRequestId = string;

export type ToolCallId = string;

export type InferenceRequest = { version: 1, run_id: RunId, conversation_id: ConversationId, credential_id: CredentialId, model_id: ModelId, input: string, };

export type InferenceErrorCode = "authentication_failed" | "payment_required" | "rate_limited" | "provider_unavailable" | "payload_too_large" | "tls_verification_failed" | "invalid_response" | "cancelled" | "internal";

export type FinishReason = "stop" | "length" | "tool_calls" | "cancelled";

export type InferenceEvent = { "type": "started", version: 1, run_id: RunId, request_id: InferenceRequestId, provider: ProviderId, model_id: ModelId, } | { "type": "text_delta", version: 1, run_id: RunId, sequence: number, text: string, } | { "type": "reasoning_summary", version: 1, run_id: RunId, sequence: number, summary: ReasoningSummary, } | { "type": "tool_call_delta", version: 1, run_id: RunId, sequence: number, call_id: ToolCallId, tool_name: string | null, arguments_delta: string, } | { "type": "usage", version: 1, run_id: RunId, input_tokens: number, cached_input_tokens: number, output_tokens: number, reasoning_output_tokens: number, } | { "type": "context_compaction", version: 1, run_id: RunId, active: boolean, } | { "type": "completed", version: 1, run_id: RunId, finish_reason: FinishReason, } | { "type": "failed", version: 1, run_id: RunId, code: InferenceErrorCode, message: string, retryable: boolean, };

export type McpServerSummary = { version: 1, id: string, name: string, transport: string, enabled: boolean, };

export type WorkspaceStatus = { version: 1, tenant_host: string, mcp_resource_url: string, mcp_servers: Array<McpServerSummary>, models_available: number, default_model_id: string | null, default_provider: string | null, chat_ready: boolean, setup_note: string, };

export type McpServerId = string;

export type McpTransport = "http" | "sse" | "stdio" | "other";

export type McpAuthType = "none" | "header" | "bearer" | "oauth" | "other";

export type McpSignInStatus = "not_signed_in" | "signed_in" | "signed_in_for_account";

export type McpPermissionMode = "allow" | "ask" | "deny";

export type McpServerUnavailableReason = "unsupported_transport" | "unsupported_auth_method" | "needs_sign_in" | "missing_device_credential" | "invalid_url";

export type McpConnectionState = "connected" | "rejected" | "unreachable" | "not_attempted";

export type McpServerView = { version: 1, id: McpServerId,
/**
 * Unique per account in DA, and the basis of this server's tool namespace.
 */
name: string, display_name: string | null, description: string | null, transport: McpTransport, url: string | null, auth_type: McpAuthType, permission_mode: McpPermissionMode, enabled: boolean,
/**
 * True only for the bundled tenant server: it has no DA row, cannot be
 * edited or deleted, and rides the signed-in session.
 */
builtin: boolean,
/**
 * The tool-id namespace segment (`mcp.<slug>.<tool>`).
 */
slug: string, unavailable: McpServerUnavailableReason | null,
/**
 * Present only when `auth_type` is [`McpAuthType::Oauth`]: whether this
 * device already holds a grant, so the surface can offer signing in or
 * signing out rather than guessing which applies.
 */
sign_in: McpSignInStatus | null,
/**
 * The client identifier this device holds for an [`McpAuthType::Oauth`]
 * row, when an operator supplied one. `None` means this device has none
 * and sign-in resolves a client on its own.
 *
 * Not confidential -- it names a registration, it does not authenticate
 * one -- so the edit sheet can show what is stored and edit it in place
 * instead of presenting an empty field over a saved value.
 */
oauth_client_id: string | null,
/**
 * Whether this device also holds the confidential value issued for
 * [`Self::oauth_client_id`], so the surface can say that leaving the
 * field blank keeps it.
 */
oauth_client_confidential: boolean, };

export type McpServerListResult = { version: 1, servers: Array<McpServerView>, };

export type McpServerInput = { name: string, display_name: string | null, description: string | null, url: string, auth_type: McpAuthType,
/**
 * Custom header name, required when `auth_type` is
 * [`McpAuthType::Header`] and ignored otherwise.
 */
header_name: string | null, permission_mode: McpPermissionMode, enabled: boolean, };

export type McpToolView = { version: 1,
/**
 * The namespaced tool id the model and the approval prompt both see.
 */
tool_id: string,
/**
 * The bare name the server advertised.
 */
name: string, description: string | null,
/**
 * Whether a call pauses for approval under this server's
 * [`McpPermissionMode`] and the tool's own annotations.
 */
requires_approval: boolean, };

export type McpConnectionReport = { version: 1, id: McpServerId, state: McpConnectionState,
/**
 * Tools advertised, when the handshake succeeded.
 */
tools: Array<McpToolView>,
/**
 * A short, already-redacted explanation for a non-`Connected` state.
 */
message: string | null, };

export type McpReauthReason = "credential_refused" | "insufficient_scope";

export type McpReauthRequired = { version: 1,
/**
 * The server to reconnect — the id `mcp_server_sign_in` takes.
 */
id: McpServerId,
/**
 * What to call it in the prompt: the row's display name when it has one,
 * otherwise its name.
 */
label: string, reason: McpReauthReason, };

export type McpServerErrorCode = "invalid_request" | "url_rejected" | "conflict" | "not_editable" | "not_found" | "unauthorized" | "storage_unavailable" | "service_unavailable" | "discovery_failed" | "client_registration_unavailable" | "sign_in_declined" | "sign_in_timed_out" | "sign_in_failed";

export type McpServerOperationError = { version: 1, code: McpServerErrorCode, };

export type ChatSubmitStatus = "submitted" | "failed";

export type ChatSubmitResult = { version: 1, status: ChatSubmitStatus, conversation_id: ConversationId | null, run_id: string | null, message: string, };

export type ChatErrorCode = "auth_required" | "not_configured" | "session_expired" | "no_active_account" | "send_in_flight" | "too_many_runs" | "not_found" | "temporarily_unavailable" | "service_unavailable" | "failed";

export type ChatOperationError = { version: 1, code: ChatErrorCode,
/**
 * User-facing prose. Always populated, and never a machine token: the
 * frontend renders it directly.
 */
message: string, };

export type ChatSyncAvailability = "live" | "stale";

export type ChatMcpSelectionView = { version: 1, server_id: McpServerId | null, };

export type ChatConversationSummary = { version: 1, conversation_id: ConversationId, title: string, updated_at: string,
/**
 * RFC 3339 pin instant, `None` when unpinned — the sidebar's "Pinned"
 * group and its recency-of-pin ordering, mirroring xfalcon-web.
 */
pinned_at?: string | null, project_id?: ProjectId | null, };

export type ChatWorkspaceBootstrap = { version: 1, conversations: Array<ChatConversationSummary>, recovery_notice?: string | null, };

export type ChatModelCatalogView = { version: 1, models: Array<ModelView>, selected_model_id: ModelId,
/**
 * True for an existing conversation. The renderer may display that model
 * but cannot replace it.
 */
selection_locked: boolean, };

export type ChatMessageFeedback = { message_id: string, rating: number, };

export type ChatFollowUpsResult = { suggestions: Array<string>, };

export type ChatMessageRole = "user" | "assistant" | "system" | "tool";

export type ChatToolCallStatus = "answered" | "unanswered";

export type ToolStepPathV1 = {
/**
 * Project-relative, re-validated at projection time — the rows a summary
 * is built from round-tripped the data plane, so the path is untrusted
 * input on the way back regardless of who wrote it.
 */
path: string,
/**
 * The classifier's own verdict, carried so a withheld or refused candidate
 * can be shown as one. Drawing it identically to an ordinary hit is the
 * single confusion this data must never create.
 */
is_sensitive: boolean, };

export type ToolStepSummaryV1 = { version: 1,
/**
 * One short phrase, this app's own: "Listed 14 entries", "Read",
 * "Blocked — a person must approve reading this file".
 */
label: string,
/**
 * The paths this step touched, in outcome order, at most
 * [`TOOL_STEP_PATH_CAP`] of them.
 */
paths: Array<ToolStepPathV1>,
/**
 * How many paths the outcome held BEFORE the display budget was applied
 * and before any path was dropped for failing validation. Carried
 * separately because a step showing twelve chips and saying nothing else
 * would read as a call that touched twelve files.
 */
path_total: number, };

export type ChatToolCallView = {
/**
 * The provider's own call id — opaque, and the only thing that pairs a
 * request to its answer.
 */
call_id: string,
/**
 * The tool name as the server advertised it.
 */
name: string, status: ChatToolCallStatus,
/**
 * The JSON argument text the turn advertised for this call, capped.
 * `None` when the durable row carried none. Defaulted so snapshots
 * sealed before this field existed still deserialize.
 */
arguments: string | null, arguments_truncated: boolean,
/**
 * The answer row's content, capped. `None` while the call is unanswered.
 */
result: string | null, result_truncated: boolean,
/**
 * What the call did, derived from the UNCAPPED outcome before
 * [`TOOL_PAYLOAD_PROJECTION_CAP_BYTES`] is applied to `result`. `None` for
 * an unanswered call, for a tool this app did not execute, and for any
 * outcome it cannot read.
 *
 * Defaulted for the same reason its siblings are — a snapshot sealed
 * before this field existed must still open, or the first launch after
 * this change destroys every offline conversation. No
 * `skip_serializing_if`: the key stays on the wire (as `null`) so one
 * shape is pinned rather than two.
 */
step: ToolStepSummaryV1 | null, };

export type ChatAttachmentView = { attachment_id: string, file_name: string, mime_type: string, };

export type ChatMessageView = { version: 1, message_id: string, role: ChatMessageRole, text: string, sequence: bigint,
/**
 * The tool round this turn performed, empty for a turn that ran none. An
 * assistant turn may carry these and no `text` — a model answering purely
 * with tool calls emits no prose, and that turn is still what the reader
 * needs to see.
 */
tool_calls: Array<ChatToolCallView>,
/**
 * The files sent with this turn, in the order they were attached. Empty
 * for a turn that carried none.
 */
attachments: Array<ChatAttachmentView>, };

export type ChatConversationView = { version: 1, conversation_id: ConversationId, title: string, availability: ChatSyncAvailability, messages: Array<ChatMessageView>, active_run_id: RunId | null, };

export type ChatProjectionEvent = { "type": "conversation_removed", version: 1, account_id: string, conversation_id: ConversationId, } | { "type": "snapshot_replaced", version: 1, conversation: ChatConversationView, } | { "type": "run_state_changed", version: 1, conversation_id: ConversationId, run_id: RunId, state: RunState, };

export type DocumentExportFormat = "markdown" | "csv" | "svg" | "html";

export type DocumentExportResult = { version: 1,
/**
 * `false` when the person dismissed the save dialog. A cancelled save is
 * a decision, not a failure, so it comes back as a successful call whose
 * answer is "nothing was written" — a caller that reported an error here
 * would accuse the user of a fault they did not commit.
 */
saved: boolean,
/**
 * The written file's name. `None` whenever `saved` is `false`.
 */
file_name: string | null, };

export type UpdateCheckResult = { version: 1, current_version: string, available_version: string | null, release_notes: string | null, is_staged: boolean, };

export type UpdateDownloadProgress = { version: 1, downloaded_bytes: number, total_bytes: number | null, percent: number | null, };

export type ToolId = string;

export type InvocationId = string;

export type ToolSource = "internal" | "mcp";

export type ToolRisk = "read_only" | "mutating";

export type ToolDescriptorView = { version: 1, id: ToolId, source: ToolSource, name: string, description: string, risk: ToolRisk, capabilities: Array<string>, idempotent: boolean, };

export type ToolInvocationStatus = "requested" | "awaiting_approval" | "approved" | "denied" | "running" | "completed" | "failed" | "cancelled";

export type ToolInvocationEvent = { "type": "requested", version: 1, invocation_id: InvocationId, run_id: RunId, tool_id: ToolId, } | { "type": "approval_required", version: 1, invocation_id: InvocationId, tool_id: ToolId, } | { "type": "approved", version: 1, invocation_id: InvocationId, } | { "type": "denied", version: 1, invocation_id: InvocationId, reason_code: string, } | { "type": "started", version: 1, invocation_id: InvocationId, } | { "type": "progress", version: 1, invocation_id: InvocationId, sequence: number, summary: string, } | { "type": "completed", version: 1, invocation_id: InvocationId, truncated: boolean, } | { "type": "failed", version: 1, invocation_id: InvocationId, message: string, retryable: boolean, } | { "type": "cancelled", version: 1, invocation_id: InvocationId, };

export type ProjectId = string;

export type ProjectTrustStatus = "trusted" | "unavailable";

export type ProjectView = { version: 1, id: ProjectId, display_name: string, root_path: string, status: ProjectTrustStatus, active: boolean, trusted_at_ms: number, };

export type ProjectListResult = { version: 1, projects: Array<ProjectView>, };

export type FolderGrantView = { version: 1, grant_id: string, display_name: string, display_path: string, };

export type ProjectFileRevisionView = { version: 1, backup_id: string, created_at_ms: number, original_bytes: number, compressed_bytes: number, content_hash: string, };

export type ProjectFileRevisionList = { version: 1, project_id: ProjectId, relative_path: string, revisions: Array<ProjectFileRevisionView>, };

export type ProjectFileRestoreResult = { version: 1, project_id: ProjectId, relative_path: string, };

export type ProjectRevisionDiffLimit = "file_size" | "change_size";

export type ProjectFileRevisionDiff = { "kind": "changes", version: 1, backup_id: string, unified_diff: string, added_lines: number, removed_lines: number, } | { "kind": "unchanged", version: 1, backup_id: string, } | { "kind": "not_text", version: 1, backup_id: string, } | { "kind": "too_large", version: 1, backup_id: string, limit: ProjectRevisionDiffLimit, } | { "kind": "sensitive_withheld", version: 1, backup_id: string, sensitive_class: ProjectSensitiveClass, };

export type ProjectEntryKind = "file" | "directory" | "symlink" | "other";

export type ProjectDirectoryEntry = { name: string, kind: ProjectEntryKind, size_bytes: number,
/**
 * Classified as sensitive by the same classifier that gates reads.
 * Flagged so the UI can mark it without exposing any content.
 */
is_sensitive: boolean, };

export type ProjectDirectoryListing = { version: 1, project_id: ProjectId,
/**
 * The listed directory, project-relative. `None` is the trusted root.
 */
path: string | null, entries: Array<ProjectDirectoryEntry>, };

export type ProjectFileMetadataReason = "binary" | "non_utf8_text" | "oversized";

export type ProjectSensitiveClass = "dot_env" | "private_key_material" | "cloud_or_kubernetes_credentials" | "password_or_vault_export" | "browser_profile_store" | "ssh_or_gpg_material" | "token_or_config_store" | "configured";

export type ProjectFileStat = { "kind": "text", version: 1, path: string, size_bytes: number, modified_ms: number | null, } | { "kind": "not_text", version: 1, path: string, size_bytes: number, modified_ms: number | null, reason: ProjectFileMetadataReason, } | { "kind": "too_large", version: 1, path: string, size_bytes: number, limit_bytes: number, } | { "kind": "sensitive_withheld", version: 1, path: string, sensitive_class: ProjectSensitiveClass, };

export type ProjectDocumentRead = { version: 1, path: string, text: string, size_bytes: number, modified_ms: number | null, };

export type ProjectOfficePreviewStarted = { version: 1, render_id: string, };

export type ProjectTableRows = { version: 1, path: string, headers: Array<string>, rows: Array<Array<string>>,
/**
 * The first data row in `rows`, counting from zero after the header.
 */
start_row: number,
/**
 * Where the next window starts; absent at the end of the file.
 */
next_row: number | null,
/**
 * The file's data-row count, once a read has reached its end.
 */
total_rows: number | null,
/**
 * A cell or a row was cut to the display bound.
 */
truncated: boolean, };

export type ProjectColumnSchema = { name: string, inferred_type: string, sample_values: Array<string>, };

export type ProjectTableSchema = { name: string, relative_path: string, format: string, columns: Array<ProjectColumnSchema>, sample_rows: Array<Array<string>>, row_count_estimate: number | null, };

export type ProjectSchemaSummary = { version: 1, project_id: ProjectId, tables: Array<ProjectTableSchema>, };

export type UploadId = string;

export type UploadPurpose = "chat_attachment" | "persistent_dataset";

export type UploadState = "selected" | "preparing" | "uploading" | "resigning" | "finalizing" | "ready" | "failed" | "cancelled";

export type UploadProjectionV1 = { version: 1, id: UploadId, purpose: UploadPurpose, state: UploadState, bytes_sent: number, size: number, display_name: string, safe_error: string | null,
/**
 * DA's chat-attachment or dataset id once the transfer is ready. The
 * webview uses this to bind a chat attachment onto the next send. It is
 * not a path, URL, or storage key.
 */
resource_id?: string | null,
/**
 * Conversation this upload belongs to, when it is a chat attachment.
 * Not a path, URL, or storage key.
 */
conversation_id?: string | null, };

export type UploadQueueV1 = { version: 1,
/**
 * Monotonic for this process. Consumers ignore an older snapshot.
 */
revision: number, uploads: Array<UploadProjectionV1>, };

export type DatasetId = string;

export type StagedDatasetId = string;

export type DatasetStatus = "uploading" | "ready" | "failed";

export type DatasetFormat = "csv" | "xlsx";

export type DatasetColumnView = { name: string, inferred_type: string, };

export type DatasetFileView = { file_id: string, table_name: string, row_count: number | null, column_count: number | null, size_bytes: number | null, original_filename: string | null,
/**
 * The server's inferred schema, in the order it returned. A map would let
 * the renderer's iteration order decide the column order.
 */
columns: Array<DatasetColumnView>, created_at_ms: bigint, updated_at_ms: bigint, };

export type DatasetView = { version: 1, id: DatasetId, display_name: string, status: DatasetStatus,
/**
 * Epoch milliseconds. DA reports an ISO-8601 string; it is parsed in the
 * backend so every surface reads the same instant the same way, rather
 * than each one carrying its own string parser.
 */
created_at_ms: bigint, updated_at_ms: bigint, };

export type DatasetListResult = { version: 1, datasets: Array<DatasetView>,
/**
 * False only when an administrator deactivated the account's upload
 * source. DA omits the field on older builds, and absence there is read
 * as enabled — so a server that never learned to report it is not
 * mistaken for one that turned uploads off.
 */
uploads_enabled: boolean,
/**
 * The page cap stopped the walk before DA's reported total. Reported so
 * the surface can say the list is partial; a truncated list presented as
 * complete is how a person concludes their dataset was lost.
 */
truncated: boolean, };

export type DatasetDetail = { version: 1, dataset: DatasetView, files: Array<DatasetFileView>,
/**
 * Same default-true semantics as [`DatasetListResult::uploads_enabled`].
 * Present here too because the drawer's download action is gated by it.
 */
uploads_enabled: boolean, };

export type StagedDatasetView = { version: 1, id: StagedDatasetId,
/**
 * The suggested dataset name, already sanitized for display. The person
 * can rename it before uploading.
 */
display_name: string, size: number, format: DatasetFormat, worksheets: Array<string>, };

export type DatasetPreviewUnavailable = "file_too_large" | "not_delimited" | "unreadable";

export type DatasetPreview = { version: 1, staged_id: StagedDatasetId,
/**
 * Which worksheet these rows came from. Zero for a format with none.
 */
worksheet_index: number,
/**
 * Which sampled row was read as the header row.
 */
header_row_index: number,
/**
 * The row the backend suggests holds the headers, so a file with a title
 * or banner row above its columns opens on the right one.
 */
suggested_header_row: number, columns: Array<DatasetColumnView>,
/**
 * A cell absent from the file is `null`, distinct from an empty string:
 * the difference decides whether a remove-nulls step drops the row.
 */
rows: Array<Array<string | null>>,
/**
 * The leading raw rows, before a header row is chosen, so the picker can
 * show what each candidate would name the columns.
 */
header_candidates: Array<Array<string | null>>,
/**
 * Rows read from the file before the recipe ran.
 */
sampled_rows: number, truncated_rows: boolean, truncated_columns: boolean,
/**
 * At least one cell was cut to its display ceiling.
 */
truncated_cell: boolean,
/**
 * Set when there are no rows to show and why. `columns` and `rows` are
 * empty whenever it is present.
 */
unavailable: DatasetPreviewUnavailable | null,
/**
 * False when the file is above the recipe's parse ceiling. The surface
 * disables the recipe controls rather than accepting steps it cannot
 * apply to the uploaded file.
 */
recipe_available: boolean, };

export type DatasetCastType = "text" | "number" | "integer" | "boolean" | "date";

export type DatasetFilterOp = "equals" | "not_equals" | "greater_than" | "less_than" | "at_least" | "at_most" | "contains" | "is_null" | "is_not_null";

export type DatasetRecipeStep = { "kind": "filter", column: string, op: DatasetFilterOp,
/**
 * Absent for the null tests, required for every other comparison.
 */
value: string | null, } | { "kind": "sort", column: string, descending: boolean, } | { "kind": "rename", column: string, new_name: string, } | { "kind": "cast", column: string, cast_to: DatasetCastType, } | { "kind": "trim", column: string, } | { "kind": "upper", column: string, } | { "kind": "lower", column: string, } | { "kind": "replace", column: string, find: string, replace_with: string, } | { "kind": "split", column: string, delimiter: string, parts: number, } | { "kind": "dedup" } | { "kind": "remove_nulls", column: string, } | { "kind": "fill_nulls", column: string, value: string, };

export type DatasetRecipe = { steps: Array<DatasetRecipeStep>, };

export type DatasetUploadRequest = { staged_id: StagedDatasetId, display_name: string,
/**
 * Omitted when the person did not choose, so DA applies its own default
 * rather than this app asserting one on their behalf.
 */
worksheet_index: number | null, header_row_index: number | null, recipe: DatasetRecipe | null, };

export type DatasetUploadStartResult = { version: 1, upload_id: UploadId, queue: UploadQueueV1, };

export type DatasetErrorCode = "auth_required" | "not_configured" | "uploads_disabled" | "update_pending" | "stage_not_found" | "stage_limit_reached" | "no_file_chosen" | "unsupported_format" | "content_mismatch" | "encrypted_file" | "empty_file" | "file_too_large" | "unreadable" | "recipe_too_long" | "recipe_invalid_step" | "recipe_file_too_large" | "not_found" | "server_refused" | "network" | "storage";

export type DatasetOperationError = { version: 1, code: DatasetErrorCode,
/**
 * Which recipe step failed, counting from zero. Present only with
 * [`DatasetErrorCode::RecipeInvalidStep`]: a step that is silently
 * skipped uploads data that differs from the reviewed preview, so the
 * position travels with the refusal.
 */
step_index: number | null, };

export type DatasetDropOutcome = { version: 1, staged: StagedDatasetView | null, error: DatasetOperationError | null, };

export type ThemePreference = "dark" | "light";

export type ReadingSize = "small" | "medium" | "large";

export type AppearanceSettings = { version: 1, theme: ThemePreference, reading_size: ReadingSize,
/**
 * Whether the transcript offers a run's streamed reasoning behind its
 * collapsed disclosure. A display preference, not a capability one: the
 * reasoning is requested, streamed and bounded either way, because it is
 * what makes a thinking model answer well (spec: chat-reasoning-panel).
 */
show_reasoning: boolean,
/**
 * Whether settled assistant answers offer generated follow-up prompts.
 */
show_follow_ups: boolean, };

export type SettingsErrorCode = "storage_unavailable";

export type SettingsOperationError = { version: 1, code: SettingsErrorCode, };

export type SkillView = { version: 1, name: string, description: string,
/**
 * The manifest's own `version`, or `0.0.0` when it declares none.
 */
declared_version: string,
/**
 * Whether runs are told about it. A disabled skill stays on disk.
 */
enabled: boolean,
/**
 * SHA-256 of the stored `SKILL.md` bytes, so a re-import that changed
 * nothing is visibly a no-op.
 */
content_hash: string,
/**
 * How many files the skill bundles beside its `SKILL.md` — references,
 * scripts, assets. Zero for a skill imported as a single file.
 */
bundled_files: number, };

export type SkillListResult = { version: 1, skills: Array<SkillView>, };

export type SkillImportFailure = {
/**
 * The file's name alone, for display: never its full path. A file nested
 * in a dropped folder has a path the webview was never given.
 */
path: string, reason: string, };

export type SkillImportResult = { version: 1, imported: Array<SkillView>, failed: Array<SkillImportFailure>, };

export type SkillErrorCode = "not_found" | "storage_unavailable" | "no_active_account" | "invalid_choice" | "selection_too_large";

export type SkillOperationError = { version: 1, code: SkillErrorCode, };

export type SkillPickKind = "files" | "folder";

export type SkillSourceGrantView = { version: 1, grant_id: string, count: number, labels: Array<string>, };

export type ReportId = string;

export type ReportScope = "user" | "global";

export type ReportKind = "dashboard" | "workbook" | "presentation";

export type ReportView = { version: 1, id: ReportId, kind: ReportKind, title: string, description: string | null, created_at_ms: bigint, updated_at_ms: bigint,
/**
 * When DA stored the report's thumbnail image, or `None` when it holds none
 * (the card then draws the report itself). A changed value is a new image,
 * so it is the key a cached thumbnail is kept under.
 */
thumbnail_updated_at_ms: bigint | null,
/**
 * The reader's own pin. Per viewer, so one user's pin never reorders
 * another user's list.
 */
pinned: boolean,
/**
 * The reader's own hide. A preference, not access control: a hidden report
 * is still listed so it can be shown again, and still opens by id.
 */
hidden: boolean,
/**
 * Total payload bytes DA holds for the report, shown as its size.
 */
byte_size: bigint, scope: ReportScope,
/**
 * Whether the signed-in account created the report. Only the owner can
 * rename or delete a user-scoped report.
 */
is_owner: boolean,
/**
 * The creator's name when the report is shared, so a reader can tell whose
 * report it is. Absent when DA does not expose one.
 */
owner_display_name: string | null,
/**
 * Whether the report can be re-run against its datasets. Set by DA when the
 * report is saved, so a report that is not refreshable never has a refresh
 * action.
 */
refreshable: boolean,
/**
 * DA's answer for this viewer. The surface shows an action only when the
 * matching flag is set, and DA still refuses it if the answer is stale.
 */
can_refresh: boolean, can_manage: boolean, can_delete: boolean, page_count: number, dataset_count: number,
/**
 * The names of the data sources the report's datasets read, as DA lists
 * those this reader can see. Empty for a report with none, or none visible.
 */
data_sources: Array<string>,
/**
 * Absent once the conversation the report came from is deleted. The
 * report itself outlives it.
 */
source_conversation_id: ConversationId | null, };

export type ReportListResult = { version: 1, reports: Array<ReportView>,
/**
 * The project each linked report was created in, for the library's Project
 * filter. Device-local: DA keeps no such fact, so a report made on another
 * device, or before the link existed, is not listed here and reads as having
 * no project. At most one entry per report.
 */
project_origins: Array<ReportProjectOrigin>, };

export type ReportProjectOrigin = { version: 1, report_id: ReportId, project_id: string, project_name: string, };

export type ReportDatasetFreshness = { version: 1, dataset_id: string,
/**
 * Absent when DA does not know: the dataset was never refreshed, so the page
 * still shows the rows it was saved with.
 */
executed_at_ms: bigint | null, row_count: bigint | null, };

export type ReportDocument = { version: 1, id: ReportId, title: string, description: string | null,
/**
 * The page the reader is viewing, by name. A multi-page report opens one
 * page at a time.
 */
page: string,
/**
 * Every page name in the report, in order, so the surface can offer the
 * rest without a second call.
 */
pages: Array<string>,
/**
 * The id the `xf-preview` scheme serves this document under. In-memory
 * and evictable, so it is resolved fresh on every open and never stored.
 */
document_id: string,
/**
 * Freshness of each dataset inlined into this page. Empty when the page
 * has none (a static report) or DA sent none.
 */
datasets: Array<ReportDatasetFreshness>, };

export type ReportRefreshOutcome = { version: 1, dataset_id: string, ok: boolean,
/**
 * Rows the dataset returned. Zero when it failed.
 */
rows: bigint,
/**
 * How long DA spent on the dataset, in milliseconds.
 */
duration_ms: bigint,
/**
 * The name of the data source DA ran it against, when DA says.
 */
data_source: string | null,
/**
 * What DA reported for a failed dataset, absent when `ok`.
 */
error: string | null, };

export type ReportRefreshResult = { version: 1, report: ReportView, outcomes: Array<ReportRefreshOutcome>, };

export type ReportThumbnailFormat = "png" | "webp";

export type ReportThumbnail = { version: 1, id: ReportId, format: ReportThumbnailFormat,
/**
 * The image bytes, standard base64 with padding.
 */
data_base64: string, };

export type ReportCaptureDocument = { version: 1, id: ReportId, document_id: string, nonce: string, };

export type ProjectReportSaveStatus = "saved" | "duplicate" | "too_large" | "not_found" | "not_html" | "not_in_project";

export type ProjectReportFile = { version: 1, project_id: ProjectId, relative_path: string, };

export type ProjectReportSaveResult = { version: 1, status: ProjectReportSaveStatus,
/**
 * The written file on `saved`, the existing one on `duplicate`, absent
 * otherwise.
 */
file: ProjectReportFile | null,
/**
 * The byte ceiling on `too_large`; absent on every other status.
 */
limit: bigint | null,
/**
 * Advisory. A project save writes the document verbatim, so a script or
 * stylesheet it cannot load stays dead in the file.
 */
external_reference: boolean, };

export type ReportErrorCode = "not_found" | "storage_unavailable" | "invalid_title" | "project_unavailable" | "forbidden" | "too_large_to_open" | "not_refreshable" | "export_failed" | "invalid_thumbnail";

export type ReportOperationError = { version: 1, code: ReportErrorCode, };

export type RendererErrorReport = { version: 1, message: string, component_stack: string | null, };

export type DiagnosticsBundleResponse = { version: 1, content: string, suggested_filename: string, };
