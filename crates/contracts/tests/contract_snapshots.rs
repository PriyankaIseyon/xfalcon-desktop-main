use schemars::schema_for;
use serde_json::{json, Value};
use ts_rs::TS;
use xfalcon_contracts::app::{
    AppBootstrap, AppHealth, AppHealthStatus, CleanupErrorCode, CleanupFailure, CleanupNextAction,
    CleanupRetryPolicy, LocalDataClearResult, LocalDataClearStatus, LocalDataResource, Platform,
};
use xfalcon_contracts::auth::{
    AuthLoginError, AuthLoginErrorCode, IdentityProjection, SessionProjection,
};
use xfalcon_contracts::credentials::{
    CredentialErrorCode, CredentialId, CredentialOperationError, CredentialStatus,
    ModelCapabilitiesView, ModelId, ModelView, ProviderCredentialView, ProviderId,
};
use xfalcon_contracts::custom_models::{
    CustomModelCreateInput, CustomModelDiscoveryInput, CustomModelDiscoveryResult,
    CustomModelDiscoveryUnavailableReason, CustomModelSampling, CustomModelUpdateInput,
    CustomModelView, DiscoveredCustomModel, ExistingCustomModel,
};
use xfalcon_contracts::datasets::{
    DatasetCastType, DatasetColumnView, DatasetDetail, DatasetDropOutcome, DatasetErrorCode,
    DatasetFileView, DatasetFilterOp, DatasetFormat, DatasetId, DatasetListResult,
    DatasetOperationError, DatasetPreview, DatasetPreviewUnavailable, DatasetRecipe,
    DatasetRecipeStep, DatasetStatus, DatasetUploadRequest, DatasetUploadStartResult, DatasetView,
    StagedDatasetId, StagedDatasetView,
};
use xfalcon_contracts::inference::{
    FinishReason, InferenceErrorCode, InferenceEvent, InferenceRequest, InferenceRequestId,
    ReasoningSummary, ToolCallId, MODEL_CONTEXT_RESERVED_TOKENS, MODEL_MAX_OUTPUT_TOKENS_PER_TURN,
};
use xfalcon_contracts::mcp::{
    McpAuthType, McpConnectionReport, McpConnectionState, McpPermissionMode, McpReauthReason,
    McpReauthRequired, McpServerErrorCode, McpServerId, McpServerInput, McpServerListResult,
    McpServerOperationError, McpServerUnavailableReason, McpServerView, McpSignInStatus,
    McpToolView, McpTransport, BUNDLED_SERVER_ID,
};
use xfalcon_contracts::plan_providers::{
    ChatgptPlanState, ChatgptPlanStatusView, ClaudeEffort, ClaudeEffortInput, ClaudeRuntimeState,
    ClaudeRuntimeStatusView, PlanEnableInput,
};
use xfalcon_contracts::projects::{
    ProjectColumnSchema, ProjectDirectoryEntry, ProjectDirectoryListing, ProjectDocumentRead,
    ProjectEntryKind, ProjectFileMetadataReason, ProjectFileRestoreResult, ProjectFileRevisionDiff,
    ProjectFileRevisionList, ProjectFileRevisionView, ProjectFileStat, ProjectId,
    ProjectListResult, ProjectOfficePreviewStarted, ProjectRevisionDiffLimit, ProjectSchemaSummary,
    ProjectSensitiveClass, ProjectTableSchema, ProjectTrustStatus, ProjectView,
};
use xfalcon_contracts::tools::{
    InvocationId, ToolDescriptorView, ToolId, ToolInvocationEvent, ToolInvocationStatus, ToolRisk,
    ToolSource,
};
use xfalcon_contracts::uploads::{
    UploadId, UploadProjectionV1, UploadPurpose, UploadQueueV1, UploadState,
};
use xfalcon_contracts::{
    AppError, AppErrorCode, ContractVersion, ConversationId, RunEvent, RunId, RunState,
    RuntimeEvent, CONTRACT_VERSION,
};

const CHECKED_IN_TYPESCRIPT: &str = include_str!("../../../src/lib/generated/contracts.ts");

fn object_keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .expect("contract should serialize as an object")
        .keys()
        .map(String::as_str)
        .collect()
}

fn normalized_line_endings(value: &str) -> String {
    value.replace("\r\n", "\n")
}

fn generated_typescript() -> String {
    let generated = [
        ContractVersion::decl(),
        AppErrorCode::decl(),
        AppError::decl(),
        RunId::decl(),
        ConversationId::decl(),
        RunState::decl(),
        RunEvent::decl(),
        RuntimeEvent::decl(),
        Platform::decl(),
        AppBootstrap::decl(),
        AppHealthStatus::decl(),
        AppHealth::decl(),
        LocalDataResource::decl(),
        CleanupErrorCode::decl(),
        CleanupRetryPolicy::decl(),
        CleanupNextAction::decl(),
        CleanupFailure::decl(),
        LocalDataClearStatus::decl(),
        LocalDataClearResult::decl(),
        IdentityProjection::decl(),
        SessionProjection::decl(),
        AuthLoginErrorCode::decl(),
        AuthLoginError::decl(),
        ProviderId::decl(),
        CredentialId::decl(),
        CredentialStatus::decl(),
        ProviderCredentialView::decl(),
        CredentialErrorCode::decl(),
        CredentialOperationError::decl(),
        ModelId::decl(),
        ModelCapabilitiesView::decl(),
        ModelView::decl(),
        CustomModelView::decl(),
        CustomModelSampling::decl(),
        CustomModelCreateInput::decl(),
        CustomModelUpdateInput::decl(),
        CustomModelDiscoveryInput::decl(),
        CustomModelDiscoveryResult::decl(),
        CustomModelDiscoveryUnavailableReason::decl(),
        DiscoveredCustomModel::decl(),
        ExistingCustomModel::decl(),
        ClaudeRuntimeState::decl(),
        ClaudeEffort::decl(),
        ClaudeRuntimeStatusView::decl(),
        PlanEnableInput::decl(),
        ClaudeEffortInput::decl(),
        ChatgptPlanState::decl(),
        ChatgptPlanStatusView::decl(),
        format!("const MODEL_CONTEXT_RESERVED_TOKENS = {MODEL_CONTEXT_RESERVED_TOKENS};"),
        format!("const MODEL_MAX_OUTPUT_TOKENS_PER_TURN = {MODEL_MAX_OUTPUT_TOKENS_PER_TURN};"),
        ReasoningSummary::decl(),
        InferenceRequestId::decl(),
        ToolCallId::decl(),
        InferenceRequest::decl(),
        InferenceErrorCode::decl(),
        FinishReason::decl(),
        InferenceEvent::decl(),
        xfalcon_contracts::workspace::McpServerSummary::decl(),
        xfalcon_contracts::workspace::WorkspaceStatus::decl(),
        McpServerId::decl(),
        McpTransport::decl(),
        McpAuthType::decl(),
        McpSignInStatus::decl(),
        McpPermissionMode::decl(),
        McpServerUnavailableReason::decl(),
        McpConnectionState::decl(),
        McpServerView::decl(),
        McpServerListResult::decl(),
        McpServerInput::decl(),
        McpToolView::decl(),
        McpConnectionReport::decl(),
        McpReauthReason::decl(),
        McpReauthRequired::decl(),
        McpServerErrorCode::decl(),
        McpServerOperationError::decl(),
        xfalcon_contracts::chat::ChatSubmitStatus::decl(),
        xfalcon_contracts::chat::ChatSubmitResult::decl(),
        xfalcon_contracts::chat::ChatErrorCode::decl(),
        xfalcon_contracts::chat::ChatOperationError::decl(),
        xfalcon_contracts::chat::ChatSyncAvailability::decl(),
        xfalcon_contracts::chat::ChatMcpSelectionView::decl(),
        xfalcon_contracts::chat::ChatConversationSummary::decl(),
        xfalcon_contracts::chat::ChatWorkspaceBootstrap::decl(),
        xfalcon_contracts::chat::ChatModelCatalogView::decl(),
        xfalcon_contracts::chat::ChatMessageFeedback::decl(),
        xfalcon_contracts::chat::ChatFollowUpsResult::decl(),
        xfalcon_contracts::chat::ChatMessageRole::decl(),
        xfalcon_contracts::chat::ChatToolCallStatus::decl(),
        xfalcon_contracts::chat::ToolStepPathV1::decl(),
        xfalcon_contracts::chat::ToolStepSummaryV1::decl(),
        xfalcon_contracts::chat::ChatToolCallView::decl(),
        xfalcon_contracts::chat::ChatAttachmentView::decl(),
        xfalcon_contracts::chat::ChatMessageView::decl(),
        xfalcon_contracts::chat::ChatConversationView::decl(),
        xfalcon_contracts::chat::ChatProjectionEvent::decl(),
        xfalcon_contracts::export::DocumentExportFormat::decl(),
        xfalcon_contracts::export::DocumentExportResult::decl(),
        xfalcon_contracts::update::UpdateCheckResult::decl(),
        xfalcon_contracts::update::UpdateDownloadProgress::decl(),
        ToolId::decl(),
        InvocationId::decl(),
        ToolSource::decl(),
        ToolRisk::decl(),
        ToolDescriptorView::decl(),
        ToolInvocationStatus::decl(),
        ToolInvocationEvent::decl(),
        ProjectId::decl(),
        ProjectTrustStatus::decl(),
        ProjectView::decl(),
        ProjectListResult::decl(),
        xfalcon_contracts::projects::FolderGrantView::decl(),
        ProjectFileRevisionView::decl(),
        ProjectFileRevisionList::decl(),
        ProjectFileRestoreResult::decl(),
        ProjectRevisionDiffLimit::decl(),
        ProjectFileRevisionDiff::decl(),
        ProjectEntryKind::decl(),
        ProjectDirectoryEntry::decl(),
        ProjectDirectoryListing::decl(),
        ProjectFileMetadataReason::decl(),
        ProjectSensitiveClass::decl(),
        ProjectFileStat::decl(),
        ProjectDocumentRead::decl(),
        ProjectOfficePreviewStarted::decl(),
        xfalcon_contracts::projects::ProjectTableRows::decl(),
        ProjectColumnSchema::decl(),
        ProjectTableSchema::decl(),
        ProjectSchemaSummary::decl(),
        UploadId::decl(),
        UploadPurpose::decl(),
        UploadState::decl(),
        UploadProjectionV1::decl(),
        UploadQueueV1::decl(),
        DatasetId::decl(),
        StagedDatasetId::decl(),
        DatasetStatus::decl(),
        DatasetFormat::decl(),
        DatasetColumnView::decl(),
        DatasetFileView::decl(),
        DatasetView::decl(),
        DatasetListResult::decl(),
        DatasetDetail::decl(),
        StagedDatasetView::decl(),
        DatasetPreviewUnavailable::decl(),
        DatasetPreview::decl(),
        DatasetCastType::decl(),
        DatasetFilterOp::decl(),
        DatasetRecipeStep::decl(),
        DatasetRecipe::decl(),
        DatasetUploadRequest::decl(),
        DatasetUploadStartResult::decl(),
        DatasetErrorCode::decl(),
        DatasetOperationError::decl(),
        DatasetDropOutcome::decl(),
        xfalcon_contracts::settings::ThemePreference::decl(),
        xfalcon_contracts::settings::ReadingSize::decl(),
        xfalcon_contracts::settings::AppearanceSettings::decl(),
        xfalcon_contracts::settings::SettingsErrorCode::decl(),
        xfalcon_contracts::settings::SettingsOperationError::decl(),
        xfalcon_contracts::skills::SkillView::decl(),
        xfalcon_contracts::skills::SkillListResult::decl(),
        xfalcon_contracts::skills::SkillImportFailure::decl(),
        xfalcon_contracts::skills::SkillImportResult::decl(),
        xfalcon_contracts::skills::SkillErrorCode::decl(),
        xfalcon_contracts::skills::SkillOperationError::decl(),
        xfalcon_contracts::skills::SkillPickKind::decl(),
        xfalcon_contracts::skills::SkillSourceGrantView::decl(),
        xfalcon_contracts::reports::ReportId::decl(),
        xfalcon_contracts::reports::ReportScope::decl(),
        xfalcon_contracts::reports::ReportKind::decl(),
        xfalcon_contracts::reports::ReportView::decl(),
        xfalcon_contracts::reports::ReportListResult::decl(),
        xfalcon_contracts::reports::ReportProjectOrigin::decl(),
        xfalcon_contracts::reports::ReportDatasetFreshness::decl(),
        xfalcon_contracts::reports::ReportDocument::decl(),
        xfalcon_contracts::reports::ReportRefreshOutcome::decl(),
        xfalcon_contracts::reports::ReportRefreshResult::decl(),
        xfalcon_contracts::reports::ReportThumbnailFormat::decl(),
        xfalcon_contracts::reports::ReportThumbnail::decl(),
        xfalcon_contracts::reports::ReportCaptureDocument::decl(),
        xfalcon_contracts::reports::ProjectReportSaveStatus::decl(),
        xfalcon_contracts::reports::ProjectReportFile::decl(),
        xfalcon_contracts::reports::ProjectReportSaveResult::decl(),
        xfalcon_contracts::reports::ReportErrorCode::decl(),
        xfalcon_contracts::reports::ReportOperationError::decl(),
        xfalcon_contracts::diagnostics::RendererErrorReport::decl(),
        xfalcon_contracts::diagnostics::DiagnosticsBundleResponse::decl(),
    ]
    .map(|declaration| format!("export {declaration}"))
    .join("\n\n");
    generated
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[test]
fn app_boundary_is_versioned_narrow_and_secret_free() {
    let bootstrap = AppBootstrap::new("0.1.0", Platform::Windows);
    let health = AppHealth::new(AppHealthStatus::Ready);
    let cleanup = LocalDataClearResult::new(
        "cleanup-1",
        vec![
            LocalDataResource::ActiveWork,
            LocalDataResource::SessionVault,
            LocalDataResource::Journal,
            LocalDataResource::LocalDatabase,
            LocalDataResource::LocalProjections,
        ],
        vec![CleanupFailure::new(
            LocalDataResource::SessionVault,
            CleanupErrorCode::VaultUnavailable,
            CleanupRetryPolicy::UserInitiated,
            CleanupNextAction::RetryClearLocalData,
        )],
    );
    let unauthenticated_cleanup = LocalDataClearResult::new(
        "cleanup-2",
        Vec::new(),
        vec![CleanupFailure::new(
            LocalDataResource::ActiveWork,
            CleanupErrorCode::NoActiveAccount,
            CleanupRetryPolicy::DoNotRetry,
            CleanupNextAction::SignIn,
        )],
    );

    let bootstrap_wire = serde_json::to_value(bootstrap).expect("bootstrap serializes");
    let health_wire = serde_json::to_value(health).expect("health serializes");
    let cleanup_wire = serde_json::to_value(cleanup).expect("cleanup serializes");

    assert_eq!(bootstrap_wire["version"], json!(1));
    assert_eq!(bootstrap_wire["application_version"], json!("0.1.0"));
    assert_eq!(bootstrap_wire["platform"], json!("windows"));
    assert_eq!(
        bootstrap_wire["session"]["status"],
        json!("unauthenticated")
    );
    assert_eq!(health_wire, json!({ "version": 1, "status": "ready" }));
    assert_eq!(cleanup_wire["correlation_id"], json!("cleanup-1"));
    assert_eq!(cleanup_wire["status"], json!("partial"));
    assert_eq!(cleanup_wire["failures"][0]["retryable"], json!(true));
    assert_eq!(
        cleanup_wire["failures"][0]["retry_policy"],
        json!("user_initiated")
    );
    assert_eq!(
        cleanup_wire["failures"][0]["next_action"],
        json!("retry_clear_local_data")
    );
    assert_eq!(
        serde_json::to_value(unauthenticated_cleanup).expect("unauthenticated cleanup serializes"),
        json!({
            "version": 1,
            "correlation_id": "cleanup-2",
            "status": "failed",
            "cleared": [],
            "failures": [{
                "resource": "active_work",
                "code": "no_active_account",
                "retryable": false,
                "retry_policy": "do_not_retry",
                "next_action": "sign_in"
            }]
        })
    );

    let future_resources = [
        LocalDataResource::ProjectData,
        LocalDataResource::UploadTemporary,
        LocalDataResource::ArtifactCache,
        LocalDataResource::Export,
        LocalDataResource::DecryptedWorkingFiles,
    ];
    assert_eq!(
        serde_json::to_value(future_resources).expect("future cleanup resources serialize"),
        json!([
            "project_data",
            "upload_temporary",
            "artifact_cache",
            "export",
            "decrypted_working_files"
        ])
    );

    let generated = generated_typescript().to_ascii_lowercase();
    for forbidden in [
        "access_token",
        "refresh_token",
        "provider_key",
        "ciphertext",
        "authorization",
        "raw_reasoning",
        "chain_of_thought",
        "id_token",
        "api_key",
        "device_key",
    ] {
        let needle = format!("\"{forbidden}\"");
        assert!(
            !generated.contains(&needle),
            "app boundary exposed forbidden field {forbidden}"
        );
    }
}

#[test]
fn session_and_identity_projections_are_versioned_and_token_free() {
    let identity = IdentityProjection::new(
        "user-1",
        "tenant-1",
        "Ada Lovelace",
        Some("ada@example.com".to_owned()),
    );
    let session = SessionProjection::authenticated(identity.clone(), 1_783_500_000_000);

    let identity_wire = serde_json::to_value(identity).expect("identity serializes");
    let session_wire = serde_json::to_value(session).expect("session serializes");

    assert_eq!(identity_wire["version"], json!(1));
    assert_eq!(session_wire["version"], json!(1));
    for forbidden in [
        "access_token",
        "refresh_token",
        "id_token",
        "authorization",
        "cookie",
    ] {
        assert!(!object_keys(&identity_wire).contains(&forbidden));
        assert!(!object_keys(&session_wire).contains(&forbidden));
        assert!(
            !generated_typescript()
                .to_ascii_lowercase()
                .contains(forbidden),
            "generated TypeScript exposed forbidden field {forbidden}"
        );
    }

    serde_json::from_value::<SessionProjection>(json!({
        "status": "authenticated",
        "version": 1,
        "identity": null,
        "expires_at_ms": 1_783_500_000_000_i64,
    }))
    .expect_err("authenticated session must require a validated identity");

    assert!(
        !SessionProjection::decl().contains("reauthentication_required"),
        "desktop session contract must match the existing web current-session model"
    );
}

#[test]
fn credential_and_model_views_structurally_exclude_secret_material() {
    let credential = ProviderCredentialView::new(
        "credential-1",
        ProviderId::OpenAi,
        "Work OpenAI",
        CredentialStatus::Ready,
        true,
    );
    let model = ModelView::new(
        "gpt-example",
        ProviderId::OpenAi,
        "GPT Example",
        ModelCapabilitiesView::new(true, true, true),
    );
    let custom_model = CustomModelView {
        version: CONTRACT_VERSION,
        id: "private-1".to_owned(),
        model_id: "custom/private-1".to_owned(),
        display_name: "Private model".to_owned(),
        base_url: "https://gateway.example.org/v1".to_owned(),
        upstream_model_id: "upstream-model".to_owned(),
        context_length: 131_072,
        max_output_tokens: Some(16_384),
        supports_tools: true,
        supports_vision: false,
        sampling: CustomModelSampling::default(),
        is_enabled: true,
        is_active: true,
    };

    let credential_wire = serde_json::to_value(credential).expect("credential serializes");
    let model_wire = serde_json::to_value(model).expect("model serializes");
    let custom_model_wire = serde_json::to_value(custom_model).expect("private model serializes");
    assert_eq!(credential_wire["version"], json!(1));
    assert_eq!(model_wire["version"], json!(1));
    assert_eq!(custom_model_wire["max_output_tokens"], json!(16_384));
    assert_eq!(credential_wire["provider"], json!("openai"));
    assert_eq!(model_wire["provider"], json!("openai"));

    let incomplete_custom_model_wire = serde_json::to_value(CustomModelView {
        version: CONTRACT_VERSION,
        id: "private-incomplete".to_owned(),
        model_id: "custom/private-incomplete".to_owned(),
        display_name: "Incomplete private model".to_owned(),
        base_url: "https://gateway.example.org/v1".to_owned(),
        upstream_model_id: "upstream-model".to_owned(),
        context_length: 131_072,
        max_output_tokens: None,
        supports_tools: true,
        supports_vision: false,
        sampling: CustomModelSampling::default(),
        is_enabled: true,
        is_active: true,
    })
    .expect("incomplete private model serializes");
    assert_eq!(
        incomplete_custom_model_wire["max_output_tokens"],
        Value::Null
    );

    let generated_views = [
        ProviderCredentialView::decl(),
        ModelView::decl(),
        CustomModelView::decl(),
    ]
    .join("\n")
    .to_ascii_lowercase();
    for forbidden in [
        "api_key",
        "ciphertext",
        "envelope",
        "nonce",
        "key_identifier",
        "device_key",
        "secret",
    ] {
        assert!(
            !object_keys(&credential_wire).contains(&forbidden),
            "credential projection exposed {forbidden}"
        );
        assert!(
            !object_keys(&model_wire).contains(&forbidden),
            "model projection exposed {forbidden}"
        );
        assert!(
            !object_keys(&custom_model_wire).contains(&forbidden),
            "private model projection exposed {forbidden}"
        );
        assert!(
            !generated_views.contains(forbidden),
            "generated view types exposed forbidden term {forbidden}"
        );
    }
}

#[test]
fn inference_request_rejects_unknown_fields_and_unsupported_versions() {
    let valid = json!({
        "version": 1,
        "run_id": "run-1",
        "conversation_id": "conversation-1",
        "credential_id": "credential-1",
        "model_id": "gpt-example",
        "input": "Summarize this dataset"
    });
    serde_json::from_value::<InferenceRequest>(valid.clone()).expect("valid request");

    let mut unknown = valid.clone();
    unknown
        .as_object_mut()
        .expect("request object")
        .insert("api_key".to_owned(), json!("must-not-cross-ipc"));
    serde_json::from_value::<InferenceRequest>(unknown)
        .expect_err("security-sensitive input must reject unknown fields");

    let mut unsupported = valid;
    unsupported["version"] = json!(2);
    let error = serde_json::from_value::<InferenceRequest>(unsupported)
        .expect_err("unsupported contract version must fail");
    assert!(error.to_string().contains("unsupported contract version 2"));
}

#[test]
fn inference_events_have_stable_provider_neutral_wire_shapes() {
    let run_id = RunId::from("run-1");
    let events = [
        InferenceEvent::started(
            run_id.clone(),
            "request-1",
            ProviderId::OpenAi,
            "gpt-example",
        ),
        InferenceEvent::text_delta(run_id.clone(), 1, "Hello"),
        InferenceEvent::reasoning_summary(
            run_id.clone(),
            2,
            ReasoningSummary::provider_designated("I compared the available evidence")
                .expect("bounded summary"),
        ),
        InferenceEvent::tool_call_delta(
            run_id.clone(),
            3,
            "call-1",
            Some("ida_query".to_owned()),
            "{\"sql\":\"select",
        ),
        InferenceEvent::usage(run_id.clone(), 12, 3, 7, 2),
        InferenceEvent::context_compaction(run_id.clone(), true),
        InferenceEvent::completed(run_id.clone(), FinishReason::Stop),
        InferenceEvent::failed(
            run_id,
            InferenceErrorCode::RateLimited,
            "Provider rate limit reached",
            true,
        ),
    ];

    let event_types: Vec<Value> = events
        .iter()
        .map(|event| serde_json::to_value(event).expect("event serializes"))
        .map(|wire| {
            assert_eq!(wire["version"], json!(1));
            wire["type"].clone()
        })
        .collect();
    assert_eq!(
        event_types,
        vec![
            json!("started"),
            json!("text_delta"),
            json!("reasoning_summary"),
            json!("tool_call_delta"),
            json!("usage"),
            json!("context_compaction"),
            json!("completed"),
            json!("failed"),
        ]
    );
}

#[test]
fn reasoning_summary_is_bounded_and_raw_reasoning_has_no_public_variant() {
    assert!(
        ReasoningSummary::provider_designated("x".repeat(4_096)).is_ok(),
        "summary at the byte limit should be accepted"
    );
    assert!(
        ReasoningSummary::provider_designated("x".repeat(4_097)).is_err(),
        "summary beyond the byte limit should be rejected"
    );

    let event_decl = InferenceEvent::decl().to_ascii_lowercase();
    for forbidden in [
        "reasoning_delta",
        "raw_reasoning",
        "chain_of_thought",
        "chain-of-thought",
    ] {
        assert!(
            !event_decl.contains(forbidden),
            "raw reasoning crossed the public contract as {forbidden}"
        );
    }
}

#[test]
fn schemas_pin_version_one_and_typescript_is_checked_in_without_drift() {
    let request_schema =
        serde_json::to_value(schema_for!(InferenceRequest)).expect("request schema");
    let event_schema = serde_json::to_value(schema_for!(InferenceEvent)).expect("event schema");
    assert!(
        request_schema.to_string().contains("\"const\":1"),
        "request schema must pin contract version one"
    );
    assert!(
        event_schema.to_string().contains("\"const\":1"),
        "event schema must pin contract version one"
    );
    assert_eq!(CONTRACT_VERSION, ContractVersion::V1);
    let generated = generated_typescript();
    // Regenerate rather than diff by hand:
    //   XFALCON_WRITE_CONTRACTS=1 cargo test -p xfalcon-contracts
    // The assertion below still runs, so a regenerating run also verifies.
    if std::env::var("XFALCON_WRITE_CONTRACTS").as_deref() == Ok("1")
        && normalized_line_endings(CHECKED_IN_TYPESCRIPT) != generated
    {
        let destination = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/lib/generated/contracts.ts");
        std::fs::write(&destination, &generated)
            .expect("checked-in contracts.ts should be writable");
        panic!(
            "regenerated {}; re-run without XFALCON_WRITE_CONTRACTS to verify",
            destination.display()
        );
    }
    assert_eq!(normalized_line_endings(CHECKED_IN_TYPESCRIPT), generated);
}

#[test]
fn shared_model_context_policy_exports_numeric_typescript_constants() {
    assert_eq!(MODEL_CONTEXT_RESERVED_TOKENS, 16_384);
    assert_eq!(MODEL_MAX_OUTPUT_TOKENS_PER_TURN, 128_000);
    let generated = generated_typescript();
    assert!(generated.contains("export const MODEL_CONTEXT_RESERVED_TOKENS = 16384;"));
    assert!(generated.contains("export const MODEL_MAX_OUTPUT_TOKENS_PER_TURN = 128000;"));
}

#[test]
fn chat_mcp_selection_view_pins_version_and_serializes_all_as_null() {
    use xfalcon_contracts::chat::ChatMcpSelectionView;

    let all = ChatMcpSelectionView::new(None);
    let all_wire = serde_json::to_value(&all).expect("selection view serializes");
    assert_eq!(all_wire, json!({"version": 1, "server_id": null}));
    assert_eq!(
        serde_json::from_value::<ChatMcpSelectionView>(all_wire).unwrap(),
        all
    );

    let specific = ChatMcpSelectionView::new(Some(McpServerId::from("tenant")));
    let specific_wire = serde_json::to_value(&specific).expect("selection view serializes");
    assert_eq!(specific_wire, json!({"version": 1, "server_id": "tenant"}));
}

#[test]
fn a_document_export_result_round_trips_and_never_carries_a_directory() {
    use xfalcon_contracts::export::DocumentExportResult;

    let written = DocumentExportResult::written("Quarterly review.md");
    let wire = serde_json::to_value(&written).expect("export serializes");
    assert_eq!(wire["version"], json!(1));
    assert_eq!(wire["saved"], json!(true));
    assert_eq!(wire["file_name"], json!("Quarterly review.md"));
    // The chosen directory stays in the backend: the renderer asked for a save
    // and gets told what was written, never where the person keeps their files.
    // Asserted as the whole key set rather than by absence, so a later field
    // carrying a path has to fail here instead of slipping past a blocklist.
    let mut keys = object_keys(&wire);
    keys.sort_unstable();
    assert_eq!(keys, vec!["file_name", "saved", "version"]);
    let restored: DocumentExportResult =
        serde_json::from_value(wire).expect("export deserializes from wire");
    assert_eq!(restored, written);

    // A dismissed dialog is a successful call with nothing written, so the
    // renderer can tell "you cancelled" apart from "the write failed".
    let cancelled = serde_json::to_value(DocumentExportResult::cancelled())
        .expect("cancelled export serializes");
    assert_eq!(cancelled["saved"], json!(false));
    assert_eq!(cancelled["file_name"], Value::Null);
}

#[test]
fn a_tool_call_view_carries_an_optional_step_summary_that_never_hides_its_bound() {
    use xfalcon_contracts::chat::{
        ChatToolCallStatus, ChatToolCallView, ToolStepPathV1, ToolStepSummaryV1, TOOL_STEP_PATH_CAP,
    };

    let paths: Vec<ToolStepPathV1> = (0..TOOL_STEP_PATH_CAP)
        .map(|index| ToolStepPathV1 {
            path: format!("src/entry-{index}.ts"),
            is_sensitive: false,
        })
        .chain(std::iter::once(ToolStepPathV1 {
            path: ".env".to_owned(),
            is_sensitive: true,
        }))
        .collect();
    let summary = ToolStepSummaryV1::new("Listed 400 entries", paths, 400);
    let summary_wire = serde_json::to_value(&summary).expect("step summary serializes");
    // Sorted for the same reason the withheld-file case sorts: the claim is the
    // key SET, which `serde_json`'s `preserve_order` feature would reorder.
    let mut summary_keys = object_keys(&summary_wire);
    summary_keys.sort_unstable();
    assert_eq!(
        summary_keys,
        vec!["label", "path_total", "paths", "version"]
    );
    assert_eq!(summary_wire["version"], json!(1));
    // The display budget never redefines the count it is reporting.
    assert_eq!(summary_wire["path_total"], json!(400));
    let mut path_keys = object_keys(&summary_wire["paths"][0]);
    path_keys.sort_unstable();
    assert_eq!(
        path_keys,
        vec!["is_sensitive", "path"],
        "a path is a leaf inside a versioned parent and carries no version of its own"
    );
    assert_eq!(
        summary_wire["paths"][TOOL_STEP_PATH_CAP]["is_sensitive"],
        json!(true)
    );
    assert_eq!(
        serde_json::from_value::<ToolStepSummaryV1>(summary_wire.clone())
            .expect("step summary deserializes"),
        summary
    );

    let mut unknown = summary_wire.clone();
    unknown
        .as_object_mut()
        .expect("summary object")
        .insert("query".to_owned(), json!("; DROP TABLE users; --"));
    serde_json::from_value::<ToolStepSummaryV1>(unknown)
        .expect_err("a step summary admits no field this app did not author");

    let mut unsupported = summary_wire.clone();
    unsupported["version"] = json!(2);
    serde_json::from_value::<ToolStepSummaryV1>(unsupported)
        .expect_err("unsupported contract version must fail");

    let view = ChatToolCallView::new(
        "call-1",
        "project_list_directory",
        ChatToolCallStatus::Answered,
    )
    .with_step(Some(summary))
    .with_result(Some("{}".to_owned()), true);
    let view_wire = serde_json::to_value(&view).expect("tool call view serializes");
    let mut view_keys = object_keys(&view_wire);
    view_keys.sort_unstable();
    assert_eq!(
        view_keys,
        vec![
            "arguments",
            "arguments_truncated",
            "call_id",
            "name",
            "result",
            "result_truncated",
            "status",
            "step",
        ]
    );
    // The two truncations are independent facts: the summary is complete
    // whenever it is present, however hard the Result panel's copy was cut.
    assert_eq!(view_wire["result_truncated"], json!(true));
    assert_eq!(view_wire["step"]["path_total"], json!(400));

    // A row written by a build with no step field still opens, and says so by
    // carrying none — the offline-conversation rule this default exists for.
    let older: ChatToolCallView = serde_json::from_value(json!({
        "call_id": "call-1",
        "name": "project_list_directory",
        "status": "answered",
    }))
    .expect("a view sealed before steps existed still deserializes");
    assert_eq!(older.step, None);
}

#[test]
fn inference_request_constructor_preserves_typed_identifiers() {
    let request = InferenceRequest::new(
        RunId::from("run-1"),
        ConversationId::from("conversation-1"),
        "credential-1",
        "gpt-example",
        "Summarize this dataset",
    );
    let wire = serde_json::to_value(request).expect("request serializes");
    assert_eq!(wire["run_id"], json!("run-1"));
    assert_eq!(wire["conversation_id"], json!("conversation-1"));
}

#[test]
fn tool_descriptor_view_rejects_unknown_fields_and_unsupported_versions() {
    let valid = json!({
        "version": 1,
        "id": "read_tool",
        "source": "internal",
        "name": "Read Tool",
        "description": "Reads something.",
        "risk": "read_only",
        "capabilities": [],
        "idempotent": true
    });
    serde_json::from_value::<ToolDescriptorView>(valid.clone()).expect("valid descriptor view");

    let mut unknown = valid.clone();
    unknown
        .as_object_mut()
        .expect("descriptor object")
        .insert("input_schema".to_owned(), json!({ "type": "object" }));
    serde_json::from_value::<ToolDescriptorView>(unknown)
        .expect_err("raw schema must never cross this projection");

    let mut unsupported = valid;
    unsupported["version"] = json!(2);
    let error = serde_json::from_value::<ToolDescriptorView>(unsupported)
        .expect_err("unsupported contract version must fail");
    assert!(error.to_string().contains("unsupported contract version 2"));

    let schema = serde_json::to_value(schema_for!(ToolDescriptorView)).expect("descriptor schema");
    assert!(
        schema.to_string().contains("\"const\":1"),
        "descriptor schema must pin contract version one"
    );
}

#[test]
fn tool_invocation_events_have_stable_wire_shapes_and_exclude_raw_payloads() {
    let invocation_id = InvocationId::from("invocation-1");
    let run_id = RunId::from("run-1");
    let tool_id = ToolId::from("read_tool");
    let events = [
        ToolInvocationEvent::requested(invocation_id.clone(), run_id, tool_id.clone()),
        ToolInvocationEvent::approval_required(invocation_id.clone(), tool_id),
        ToolInvocationEvent::approved(invocation_id.clone()),
        ToolInvocationEvent::denied(invocation_id.clone(), "tool_not_allowlisted"),
        ToolInvocationEvent::started(invocation_id.clone()),
        ToolInvocationEvent::progress(invocation_id.clone(), 2, "halfway"),
        ToolInvocationEvent::completed(invocation_id.clone(), false),
        ToolInvocationEvent::failed(invocation_id.clone(), "provider unavailable", true),
        ToolInvocationEvent::cancelled(invocation_id),
    ];

    let event_types: Vec<Value> = events
        .iter()
        .map(|event| serde_json::to_value(event).expect("event serializes"))
        .map(|wire| {
            assert_eq!(wire["version"], json!(1));
            wire["type"].clone()
        })
        .collect();
    assert_eq!(
        event_types,
        vec![
            json!("requested"),
            json!("approval_required"),
            json!("approved"),
            json!("denied"),
            json!("started"),
            json!("progress"),
            json!("completed"),
            json!("failed"),
            json!("cancelled"),
        ]
    );

    let event_schema =
        serde_json::to_value(schema_for!(ToolInvocationEvent)).expect("event schema");
    assert!(
        event_schema.to_string().contains("\"const\":1"),
        "invocation event schema must pin contract version one"
    );

    let generated = generated_typescript().to_ascii_lowercase();
    for forbidden in [
        "input_schema",
        "invocation_arguments",
        "structured",
        "content_refs",
        "raw_schema",
    ] {
        assert!(
            !generated.contains(forbidden),
            "tool contracts exposed forbidden raw payload field {forbidden}"
        );
    }
}

#[test]
fn tool_source_is_a_closed_two_variant_enum() {
    // Provenance is "desktop-implemented" vs "an MCP server said so" and
    // nothing finer: which server a tool came from travels in the namespaced
    // ToolId, so this stays Copy and closed rather than growing an open
    // server-id field that every provenance site would have to carry.
    assert_eq!(
        serde_json::to_value(ToolSource::Internal).expect("source serializes"),
        json!("internal")
    );
    assert_eq!(
        serde_json::to_value(ToolSource::Mcp).expect("source serializes"),
        json!("mcp")
    );
}

#[test]
fn tool_risk_gates_mutation_as_a_closed_two_variant_enum() {
    assert_eq!(
        serde_json::to_value(ToolRisk::ReadOnly).expect("risk serializes"),
        json!("read_only")
    );
    assert_eq!(
        serde_json::to_value(ToolRisk::Mutating).expect("risk serializes"),
        json!("mutating")
    );
}

#[test]
fn tool_invocation_status_is_the_closed_lifecycle_set() {
    for (status, expected) in [
        (ToolInvocationStatus::Requested, "requested"),
        (ToolInvocationStatus::AwaitingApproval, "awaiting_approval"),
        (ToolInvocationStatus::Approved, "approved"),
        (ToolInvocationStatus::Denied, "denied"),
        (ToolInvocationStatus::Running, "running"),
        (ToolInvocationStatus::Completed, "completed"),
        (ToolInvocationStatus::Failed, "failed"),
        (ToolInvocationStatus::Cancelled, "cancelled"),
    ] {
        assert_eq!(
            serde_json::to_value(status).expect("status serializes"),
            json!(expected)
        );
    }
}

#[test]
fn project_view_pins_version_rejects_unknown_fields_and_exposes_no_raw_identity() {
    let view = ProjectView::new(
        "project-1",
        "My Project",
        "/home/user/my-project",
        ProjectTrustStatus::Trusted,
        true,
        1_783_500_000_000_i64,
    );
    let wire = serde_json::to_value(&view).expect("project view serializes");
    assert_eq!(wire["version"], json!(1));
    assert_eq!(wire["id"], json!("project-1"));
    assert_eq!(wire["status"], json!("trusted"));
    assert_eq!(wire["trusted_at_ms"], json!(1_783_500_000_000_i64));

    let mut unknown = wire.clone();
    unknown
        .as_object_mut()
        .expect("project object")
        .insert("root_handle".to_owned(), json!("must-not-cross-ipc"));
    serde_json::from_value::<ProjectView>(unknown)
        .expect_err("raw handle/identity fields must never cross this projection");

    let list = ProjectListResult::new(vec![view]);
    let list_wire = serde_json::to_value(list).expect("project list serializes");
    assert_eq!(list_wire["version"], json!(1));
    assert_eq!(list_wire["projects"][0]["id"], json!("project-1"));

    let schema = serde_json::to_value(schema_for!(ProjectView)).expect("project view schema");
    assert!(
        schema.to_string().contains("\"const\":1"),
        "project view schema must pin contract version one"
    );

    let generated = generated_typescript().to_ascii_lowercase();
    for forbidden in [
        "device_id",
        "inode",
        "volume_serial",
        "file_index",
        "root_handle",
        "cap_std",
    ] {
        assert!(
            !generated.contains(forbidden),
            "project contracts exposed forbidden internal identity field {forbidden}"
        );
    }
}

#[test]
fn project_trust_status_is_the_closed_two_variant_set() {
    assert_eq!(
        serde_json::to_value(ProjectTrustStatus::Trusted).expect("status serializes"),
        json!("trusted")
    );
    assert_eq!(
        serde_json::to_value(ProjectTrustStatus::Unavailable).expect("status serializes"),
        json!("unavailable")
    );
}

#[test]
fn mcp_server_boundary_is_versioned_and_carries_no_secret_material() {
    let view = McpServerView {
        version: CONTRACT_VERSION,
        id: McpServerId::from("9f1c2b7e-0000-4000-8000-000000000001"),
        name: "crm".to_owned(),
        display_name: Some("Acme CRM".to_owned()),
        description: None,
        transport: McpTransport::Http,
        url: Some("https://mcp.example.test/mcp".to_owned()),
        auth_type: McpAuthType::Bearer,
        permission_mode: McpPermissionMode::Ask,
        enabled: true,
        builtin: false,
        slug: "crm".to_owned(),
        unavailable: None,
        sign_in: None,
        oauth_client_id: None,
        oauth_client_confidential: false,
    };
    let wire = serde_json::to_value(&view).expect("server view serializes");
    assert_eq!(wire["version"], json!(1));
    assert_eq!(wire["auth_type"], json!("bearer"));
    assert_eq!(wire["permission_mode"], json!("ask"));
    assert_eq!(
        serde_json::from_value::<McpServerView>(wire.clone()).expect("server view deserializes"),
        view
    );

    // The projection must never grow a field that could carry a secret, a
    // bearer, or DA's web-KEK `auth_config` ciphertext.
    for forbidden in [
        "secret",
        "auth_config",
        "token",
        "password",
        "ciphertext",
        "header_value",
    ] {
        assert!(
            !object_keys(&wire).contains(&forbidden),
            "McpServerView exposed forbidden field {forbidden}"
        );
    }
    let input_declaration = McpServerInput::decl().to_ascii_lowercase();
    for forbidden in ["secret", "auth_config", "token", "password"] {
        assert!(
            !input_declaration.contains(forbidden),
            "McpServerInput exposed forbidden field {forbidden}"
        );
    }

    // The client identifier a device holds IS reported, so an edit sheet can
    // show a saved value instead of an empty field. It names a registration; it
    // does not authenticate one. Its confidential half is reported only as a
    // boolean, and the view must never gain a field that could carry the value
    // itself -- which the `forbidden` sweep above already enforces.
    let held = McpServerView {
        auth_type: McpAuthType::Oauth,
        sign_in: Some(McpSignInStatus::SignedIn),
        oauth_client_id: Some("issued-client".to_owned()),
        oauth_client_confidential: true,
        ..view
    };
    let wire = serde_json::to_value(&held).expect("server view serializes");
    assert_eq!(wire["oauth_client_id"], json!("issued-client"));
    assert_eq!(wire["oauth_client_confidential"], json!(true));
    assert_eq!(
        serde_json::from_value::<McpServerView>(wire).expect("server view deserializes"),
        held
    );
}

#[test]
fn mcp_transport_and_auth_vocabularies_match_da_and_gate_on_device_secrets() {
    // The full DA vocabulary round-trips even though only `http` is dialed:
    // a row registered elsewhere must be listed honestly, not dropped.
    for (transport, expected) in [
        (McpTransport::Http, "http"),
        (McpTransport::Sse, "sse"),
        (McpTransport::Stdio, "stdio"),
        (McpTransport::Other, "other"),
    ] {
        assert_eq!(
            serde_json::to_value(transport).expect("transport serializes"),
            json!(expected)
        );
    }
    for (auth, expected) in [
        (McpAuthType::None, "none"),
        (McpAuthType::Header, "header"),
        (McpAuthType::Bearer, "bearer"),
        (McpAuthType::Oauth, "oauth"),
        (McpAuthType::Other, "other"),
    ] {
        assert_eq!(
            serde_json::to_value(auth).expect("auth type serializes"),
            json!(expected)
        );
    }
    assert!(McpAuthType::Header.needs_device_credential());
    assert!(McpAuthType::Bearer.needs_device_credential());
    assert!(!McpAuthType::None.needs_device_credential());
    assert!(McpAuthType::None.is_supported());
    assert!(McpAuthType::Header.is_supported());
    assert!(McpAuthType::Bearer.is_supported());
    assert!(!McpAuthType::Other.is_supported());

    // `oauth` is dialable, and its device-held material is obtained rather than
    // pasted: a surface that asked for a value to type would ask the wrong
    // question, so the two predicates must not agree about it.
    assert!(McpAuthType::Oauth.is_supported());
    assert!(McpAuthType::Oauth.needs_sign_in());
    assert!(!McpAuthType::Oauth.needs_device_credential());
    for method in [
        McpAuthType::None,
        McpAuthType::Header,
        McpAuthType::Bearer,
        McpAuthType::Other,
    ] {
        assert!(
            !method.needs_sign_in(),
            "{method:?} is not established by a browser flow"
        );
    }
    for (status, expected) in [
        (McpSignInStatus::NotSignedIn, "not_signed_in"),
        (McpSignInStatus::SignedIn, "signed_in"),
    ] {
        assert_eq!(
            serde_json::to_value(status).expect("status serializes"),
            json!(expected)
        );
    }
    assert_eq!(
        serde_json::to_value(McpServerUnavailableReason::NeedsSignIn).expect("reason serializes"),
        json!("needs_sign_in")
    );
}

#[test]
fn mcp_permission_mode_defaults_to_ask() {
    assert_eq!(McpPermissionMode::default(), McpPermissionMode::Ask);
    assert_eq!(
        serde_json::to_value(McpPermissionMode::Deny).expect("mode serializes"),
        json!("deny")
    );
}

#[test]
fn bundled_server_id_is_reserved_and_recognizable() {
    assert_eq!(BUNDLED_SERVER_ID, "tenant");
    assert!(McpServerId::from(BUNDLED_SERVER_ID).is_bundled());
    assert!(!McpServerId::from("9f1c2b7e-0000-4000-8000-000000000001").is_bundled());
}

#[test]
fn mcp_connection_report_and_error_are_redacted_projections() {
    let connected = McpConnectionReport::connected(
        "tenant",
        vec![McpToolView::new(
            "mcp.tenant.ida_query",
            "ida_query",
            Some("Run a read-only analytical query".to_owned()),
            false,
        )],
    );
    let wire = serde_json::to_value(&connected).expect("report serializes");
    assert_eq!(wire["state"], json!("connected"));
    assert_eq!(wire["tools"][0]["tool_id"], json!("mcp.tenant.ida_query"));
    assert_eq!(wire["tools"][0]["requires_approval"], json!(false));
    assert_eq!(wire["message"], Value::Null);

    let failed = McpConnectionReport::failed(
        "crm",
        McpConnectionState::Rejected,
        "This server refused the registered credential.",
    );
    let failed_wire = serde_json::to_value(&failed).expect("report serializes");
    assert_eq!(failed_wire["state"], json!("rejected"));
    assert_eq!(failed_wire["tools"], json!([]));

    let error = McpServerOperationError::new(McpServerErrorCode::UrlRejected);
    assert_eq!(
        serde_json::to_value(&error).expect("error serializes"),
        json!({ "version": 1, "code": "url_rejected" })
    );
    assert_eq!(
        serde_json::to_value(McpServerUnavailableReason::MissingDeviceCredential)
            .expect("reason serializes"),
        json!("missing_device_credential")
    );
}

#[test]
fn project_file_boundary_carries_text_only_and_withholds_secrets_entirely() {
    let listing = ProjectDirectoryListing::new(
        "01HZY000000000000000000000",
        Some("src".to_owned()),
        vec![ProjectDirectoryEntry {
            name: ".env".to_owned(),
            kind: ProjectEntryKind::File,
            size_bytes: 42,
            is_sensitive: true,
        }],
    );
    let listing_wire = serde_json::to_value(&listing).expect("listing serializes");
    assert_eq!(listing_wire["version"], json!(1));
    assert_eq!(listing_wire["entries"][0]["is_sensitive"], json!(true));
    assert_eq!(listing_wire["entries"][0]["kind"], json!("file"));

    let text = ProjectFileStat::Text {
        version: CONTRACT_VERSION,
        path: "big.log".to_owned(),
        size_bytes: 5 * 1024 * 1024 * 1024,
        modified_ms: Some(1_790_000_000_000),
    };
    let text_wire = serde_json::to_value(&text).expect("stat serializes");
    assert_eq!(text_wire["kind"], json!("text"));
    // Sizes are plain numbers: a 5 GiB file is well inside the safe range.
    assert_eq!(text_wire["size_bytes"], json!(5_u64 * 1024 * 1024 * 1024));
    assert_eq!(
        serde_json::from_value::<ProjectFileStat>(text_wire).expect("stat deserializes"),
        text
    );

    let not_text = ProjectFileStat::NotText {
        version: CONTRACT_VERSION,
        path: "logo.png".to_owned(),
        size_bytes: 9_001,
        modified_ms: None,
        reason: ProjectFileMetadataReason::Binary,
    };
    let not_text_wire = serde_json::to_value(&not_text).expect("not-text serializes");
    assert_eq!(not_text_wire["kind"], json!("not_text"));
    assert_eq!(not_text_wire["reason"], json!("binary"));

    let too_large = serde_json::to_value(ProjectFileStat::TooLarge {
        version: CONTRACT_VERSION,
        path: "dump.sql".to_owned(),
        size_bytes: 12 * 1024 * 1024 * 1024,
        limit_bytes: 10 * 1024 * 1024 * 1024,
    })
    .expect("too-large serializes");
    assert_eq!(too_large["kind"], json!("too_large"));

    // A withheld secret leaks nothing measurable: no content, no hash, no size,
    // no time.
    let withheld = ProjectFileStat::SensitiveWithheld {
        version: CONTRACT_VERSION,
        path: ".env".to_owned(),
        sensitive_class: ProjectSensitiveClass::DotEnv,
    };
    let withheld_wire = serde_json::to_value(&withheld).expect("withheld serializes");
    // Sorted, because `serde_json`'s `preserve_order` feature (which feature
    // unification can switch on) changes map iteration from sorted to insertion
    // order. The claim under test is the exact key *set*, not its order.
    let mut withheld_keys = object_keys(&withheld_wire);
    withheld_keys.sort_unstable();
    assert_eq!(
        withheld_keys,
        vec!["kind", "path", "sensitive_class", "version"]
    );
    assert_eq!(withheld_wire["sensitive_class"], json!("dot_env"));

    let document = ProjectDocumentRead {
        version: CONTRACT_VERSION,
        path: "brief.docx".to_owned(),
        text: "[Office text extraction]\n\nBody".to_owned(),
        size_bytes: 512,
        modified_ms: Some(1_790_000_000_000),
    };
    let document_wire = serde_json::to_value(&document).expect("document read serializes");
    assert_eq!(document_wire["version"], json!(1));
    assert_eq!(document_wire["path"], json!("brief.docx"));
    assert_eq!(
        document_wire["text"],
        json!("[Office text extraction]\n\nBody")
    );
    assert_eq!(document_wire["size_bytes"], json!(512));
    assert_eq!(
        serde_json::from_value::<ProjectDocumentRead>(document_wire)
            .expect("document read deserializes"),
        document
    );
}

#[test]
fn appearance_defaults_to_light_small_and_hidden_reasoning_and_survives_a_partial_file() {
    use xfalcon_contracts::settings::{AppearanceSettings, ReadingSize, ThemePreference};

    let defaults = AppearanceSettings::default();
    assert_eq!(defaults.theme, ThemePreference::Light);
    assert!(!defaults.show_reasoning);
    assert!(defaults.show_follow_ups);
    assert_eq!(defaults.reading_size, ReadingSize::Small);

    let wire = serde_json::to_value(defaults).expect("appearance serializes");
    assert_eq!(wire["version"], json!(1));
    assert_eq!(wire["theme"], json!("light"));
    assert_eq!(wire["reading_size"], json!("small"));

    // A file written by an older build carries only what that build knew. The
    // read must fill the rest in rather than fail and leave nothing to render.
    let partial: AppearanceSettings =
        serde_json::from_value(json!({ "version": 1, "theme": "light" }))
            .expect("a partial settings file still reads");
    assert_eq!(partial.theme, ThemePreference::Light);
    assert_eq!(partial.reading_size, ReadingSize::Small);
    assert!(!partial.show_reasoning);
    assert!(partial.show_follow_ups);
    let explicit: AppearanceSettings = serde_json::from_value(json!({
        "version": 1, "theme": "dark", "reading_size": "large", "show_reasoning": true,
        "show_follow_ups": false
    }))
    .expect("explicit stored preferences still read");
    assert_eq!(explicit.theme, ThemePreference::Dark);
    assert_eq!(explicit.reading_size, ReadingSize::Large);
    assert!(explicit.show_reasoning);
    assert!(!explicit.show_follow_ups);
}

#[test]
fn a_skill_view_carries_metadata_and_never_its_body() {
    use xfalcon_contracts::skills::{SkillImportFailure, SkillImportResult, SkillView};

    let view = SkillView {
        version: CONTRACT_VERSION,
        name: "ida-dashboard".to_owned(),
        description: "Build self-contained interactive HTML dashboards.".to_owned(),
        declared_version: "0.0.0".to_owned(),
        enabled: true,
        content_hash: "b".repeat(64),
        bundled_files: 7,
    };
    let wire = serde_json::to_value(&view).expect("skill serializes");
    let mut keys = object_keys(&wire);
    keys.sort_unstable();
    // The body is the point: a catalog entry must never carry the
    // instructions, or progressive disclosure is decorative.
    assert_eq!(
        keys,
        vec![
            "bundled_files",
            "content_hash",
            "declared_version",
            "description",
            "enabled",
            "name",
            "version"
        ]
    );

    let result = SkillImportResult::new(
        vec![view],
        vec![SkillImportFailure {
            path: "broken.md".to_owned(),
            reason: "frontmatter delimiters not found".to_owned(),
        }],
    );
    let result_wire = serde_json::to_value(&result).expect("import result serializes");
    assert_eq!(result_wire["imported"].as_array().map(Vec::len), Some(1));
    assert_eq!(result_wire["failed"][0]["path"], json!("broken.md"));
}

#[test]
fn a_selection_too_large_refusal_names_its_code_on_the_wire() {
    use xfalcon_contracts::skills::{SkillErrorCode, SkillOperationError};

    let wire = serde_json::to_value(SkillOperationError::new(SkillErrorCode::SelectionTooLarge))
        .expect("skill error serializes");
    assert_eq!(wire["code"], json!("selection_too_large"));
}

#[test]
fn a_report_view_carries_its_identity_and_never_its_document() {
    use xfalcon_contracts::reports::{
        ReportDatasetFreshness, ReportDocument, ReportId, ReportKind, ReportListResult,
        ReportProjectOrigin, ReportScope, ReportView,
    };

    let view = ReportView {
        version: CONTRACT_VERSION,
        id: ReportId::from("report-1"),
        kind: ReportKind::Dashboard,
        title: "Q3 pipeline".to_owned(),
        description: Some("Bookings by region".to_owned()),
        created_at_ms: 1_783_500_000_000,
        updated_at_ms: 1_783_500_000_000,
        thumbnail_updated_at_ms: Some(1_783_500_000_000),
        pinned: false,
        hidden: false,
        byte_size: 48_000,
        scope: ReportScope::User,
        is_owner: true,
        owner_display_name: None,
        refreshable: true,
        can_refresh: true,
        can_manage: true,
        can_delete: true,
        page_count: 1,
        dataset_count: 2,
        data_sources: vec!["Sales warehouse".to_owned()],
        source_conversation_id: Some(ConversationId::from("conv-1")),
    };
    let wire = serde_json::to_value(&view).expect("report serializes");
    let mut keys = object_keys(&wire);
    keys.sort_unstable();
    // No `html`, `cached_html`, or `document` key. The page is fetched by the
    // backend and reaches the webview only as a preview-document id, and the
    // absence of a markup-carrying key here is what that promise looks like on
    // the wire.
    assert_eq!(
        keys,
        vec![
            "byte_size",
            "can_delete",
            "can_manage",
            "can_refresh",
            "created_at_ms",
            "data_sources",
            "dataset_count",
            "description",
            "hidden",
            "id",
            "is_owner",
            "kind",
            "owner_display_name",
            "page_count",
            "pinned",
            "refreshable",
            "scope",
            "source_conversation_id",
            "thumbnail_updated_at_ms",
            "title",
            "updated_at_ms",
            "version"
        ]
    );
    assert_eq!(wire["scope"], json!("user"));
    assert_eq!(wire["kind"], json!("dashboard"));

    // A report outlives the conversation it came from: its origin reads as
    // absent, not as an error, once that conversation is deleted.
    let orphaned = ReportView {
        source_conversation_id: None,
        ..view.clone()
    };
    let orphaned_wire = serde_json::to_value(&orphaned).expect("report serializes");
    assert_eq!(orphaned_wire["source_conversation_id"], json!(null));

    let opened = ReportDocument {
        version: CONTRACT_VERSION,
        id: ReportId::from("report-1"),
        title: "Q3 pipeline".to_owned(),
        description: None,
        page: "index.html".to_owned(),
        pages: vec!["index.html".to_owned(), "detail.html".to_owned()],
        document_id: "c".repeat(64),
        datasets: vec![ReportDatasetFreshness {
            version: CONTRACT_VERSION,
            dataset_id: "sales".to_owned(),
            executed_at_ms: Some(1_783_500_000_000),
            row_count: Some(42),
        }],
    };
    let opened_wire = serde_json::to_value(&opened).expect("document serializes");
    let mut opened_keys = object_keys(&opened_wire);
    opened_keys.sort_unstable();
    assert_eq!(
        opened_keys,
        vec![
            "datasets",
            "description",
            "document_id",
            "id",
            "page",
            "pages",
            "title",
            "version"
        ]
    );
    assert_eq!(opened_wire["pages"], json!(["index.html", "detail.html"]));
    assert_eq!(opened_wire["datasets"][0]["row_count"], json!(42));
    // A dataset DA never stored reads as unknown, not as zero.
    let unrefreshed = serde_json::to_value(ReportDatasetFreshness {
        version: CONTRACT_VERSION,
        dataset_id: "returns".to_owned(),
        executed_at_ms: None,
        row_count: None,
    })
    .expect("freshness serializes");
    assert_eq!(unrefreshed["executed_at_ms"], json!(null));
    assert_eq!(unrefreshed["row_count"], json!(null));

    let listed = ReportListResult::new(vec![view.clone()]);
    let listed_wire = serde_json::to_value(&listed).expect("list serializes");
    assert_eq!(listed_wire["reports"].as_array().map(Vec::len), Some(1));
    let mut listed_keys = object_keys(&listed_wire);
    listed_keys.sort_unstable();
    assert_eq!(listed_keys, vec!["project_origins", "reports", "version"]);
    // No project is the empty list, never a missing key.
    assert_eq!(listed_wire["project_origins"], json!([]));

    // A link names the report and the project by id and carries the project's
    // name for display. It carries no path.
    let origin = serde_json::to_value(ReportProjectOrigin::new(
        ReportId::from("report-1"),
        "project-1".to_owned(),
        "Sales".to_owned(),
    ))
    .expect("origin serializes");
    let mut origin_keys = object_keys(&origin);
    origin_keys.sort_unstable();
    assert_eq!(
        origin_keys,
        vec!["project_id", "project_name", "report_id", "version"]
    );
    let linked = serde_json::to_value(
        ReportListResult::new(vec![view.clone()]).with_project_origins(vec![
            ReportProjectOrigin::new(
                ReportId::from("report-1"),
                "project-1".to_owned(),
                "Sales".to_owned(),
            ),
        ]),
    )
    .expect("linked list serializes");
    assert_eq!(linked["project_origins"][0]["project_name"], json!("Sales"));
}

#[test]
fn a_project_report_save_names_a_project_relative_file_and_never_a_folder() {
    use xfalcon_contracts::reports::{
        ProjectReportFile, ProjectReportSaveResult, ProjectReportSaveStatus,
    };

    let file = ProjectReportFile::new(
        ProjectId::from("project-1"),
        "reports/Q3 revenue.html".to_owned(),
    );
    let saved = serde_json::to_value(ProjectReportSaveResult::saved(file.clone(), true))
        .expect("serializes");
    assert_eq!(saved["status"], json!("saved"));
    assert_eq!(saved["external_reference"], json!(true));
    // The whole key set, so a later field carrying the project's own location
    // fails here instead of slipping past a blocklist.
    let mut file_keys = object_keys(&saved["file"]);
    file_keys.sort_unstable();
    assert_eq!(file_keys, vec!["project_id", "relative_path", "version"]);
    assert_eq!(
        saved["file"]["relative_path"],
        json!("reports/Q3 revenue.html")
    );
    let restored: ProjectReportSaveResult =
        serde_json::from_value(saved).expect("deserializes from wire");
    assert_eq!(restored.file, Some(file.clone()));

    let duplicate =
        serde_json::to_value(ProjectReportSaveResult::duplicate(file)).expect("serializes");
    assert_eq!(duplicate["status"], json!("duplicate"));
    assert_eq!(duplicate["file"]["project_id"], json!("project-1"));

    let oversized =
        serde_json::to_value(ProjectReportSaveResult::too_large(1024 * 1024)).expect("serializes");
    assert_eq!(oversized["status"], json!("too_large"));
    assert_eq!(oversized["limit"], json!(1024 * 1024));
    assert_eq!(oversized["file"], json!(null));

    for (status, wire) in [
        (ProjectReportSaveStatus::NotFound, "not_found"),
        (ProjectReportSaveStatus::NotHtml, "not_html"),
        (ProjectReportSaveStatus::NotInProject, "not_in_project"),
    ] {
        let refused =
            serde_json::to_value(ProjectReportSaveResult::refused(status)).expect("serializes");
        assert_eq!(refused["status"], json!(wire));
        assert_eq!(refused["file"], json!(null));
    }
}

#[test]
fn a_report_operation_error_names_its_code_on_the_wire() {
    use xfalcon_contracts::reports::{ReportErrorCode, ReportOperationError};

    // Operation failures are the channel for a call that could not complete.
    for (code, wire) in [
        (ReportErrorCode::NotFound, "not_found"),
        (ReportErrorCode::StorageUnavailable, "storage_unavailable"),
        (ReportErrorCode::InvalidTitle, "invalid_title"),
        (ReportErrorCode::ProjectUnavailable, "project_unavailable"),
        (ReportErrorCode::Forbidden, "forbidden"),
        (ReportErrorCode::TooLargeToOpen, "too_large_to_open"),
        (ReportErrorCode::NotRefreshable, "not_refreshable"),
        (ReportErrorCode::ExportFailed, "export_failed"),
        (ReportErrorCode::InvalidThumbnail, "invalid_thumbnail"),
    ] {
        let error = serde_json::to_value(ReportOperationError::new(code)).expect("serializes");
        assert_eq!(error["code"], json!(wire));
        assert_eq!(error["version"], json!(1));
    }
}

#[test]
fn diagnostics_contracts_serialize_and_pin_version_one() {
    use xfalcon_contracts::diagnostics::{DiagnosticsBundleResponse, RendererErrorReport};

    let report = RendererErrorReport::new(
        "TypeError: undefined is not a function",
        Some("in Component\n    in App".to_string()),
    );
    let wire_report = serde_json::to_value(&report).expect("report serializes");
    assert_eq!(wire_report["version"], json!(1));
    assert_eq!(
        wire_report["message"],
        json!("TypeError: undefined is not a function")
    );
    assert_eq!(
        wire_report["component_stack"],
        json!("in Component\n    in App")
    );

    let bundle = DiagnosticsBundleResponse::new(
        "# Diagnostics Bundle\n\nVersion: 0.5.0\n",
        "xfalcon-diagnostics-20260819.txt",
    );
    let wire_bundle = serde_json::to_value(&bundle).expect("bundle serializes");
    assert_eq!(wire_bundle["version"], json!(1));
    assert_eq!(
        wire_bundle["content"],
        json!("# Diagnostics Bundle\n\nVersion: 0.5.0\n")
    );
    assert_eq!(
        wire_bundle["suggested_filename"],
        json!("xfalcon-diagnostics-20260819.txt")
    );
}

#[test]
fn custom_model_patch_distinguishes_omitted_fields_from_explicit_null() {
    let omitted: CustomModelUpdateInput =
        serde_json::from_value(json!({})).expect("an empty patch leaves fields unchanged");
    assert!(omitted.base_url.is_none());
    assert!(omitted.is_enabled.is_none());
    assert!(omitted.sampling.is_none());

    for field in [
        "base_url",
        "secret",
        "is_enabled",
        "is_active",
        "upstream_model_id",
        "display_name",
        "context_length",
        "max_output_tokens",
        "supports_tools",
        "supports_vision",
        "sampling",
    ] {
        let mut patch = serde_json::Map::new();
        patch.insert(field.to_owned(), Value::Null);
        assert!(
            serde_json::from_value::<CustomModelUpdateInput>(Value::Object(patch)).is_err(),
            "explicit null for {field} must not be treated as an omitted patch field"
        );
    }
}

#[test]
fn custom_model_sampling_rejects_out_of_range_values() {
    for sampling in [
        json!({"temperature": -0.1, "top_p": null}),
        json!({"temperature": 2.1, "top_p": null}),
        json!({"temperature": null, "top_p": -0.1}),
        json!({"temperature": null, "top_p": 1.1}),
    ] {
        assert!(serde_json::from_value::<CustomModelSampling>(sampling).is_err());
    }
    assert_eq!(
        serde_json::from_value::<CustomModelSampling>(json!({
            "temperature": 0.6,
            "top_p": 0.95
        }))
        .expect("valid sampling"),
        CustomModelSampling {
            temperature: Some(0.6),
            top_p: Some(0.95),
        }
    );
}

#[test]
fn a_refresh_result_round_trips() {
    use xfalcon_contracts::reports::{
        ReportId, ReportKind, ReportRefreshOutcome, ReportRefreshResult, ReportScope, ReportView,
    };

    let refreshed = ReportRefreshResult {
        version: CONTRACT_VERSION,
        report: ReportView {
            version: CONTRACT_VERSION,
            id: ReportId::from("report-1"),
            kind: ReportKind::Workbook,
            title: "Q3 pipeline".to_owned(),
            description: None,
            created_at_ms: 1,
            updated_at_ms: 2,
            thumbnail_updated_at_ms: None,
            pinned: true,
            hidden: true,
            byte_size: 10,
            scope: ReportScope::Global,
            is_owner: false,
            owner_display_name: Some("Ada".to_owned()),
            refreshable: true,
            can_refresh: false,
            can_manage: false,
            can_delete: false,
            page_count: 1,
            dataset_count: 2,
            data_sources: Vec::new(),
            source_conversation_id: None,
        },
        outcomes: vec![
            ReportRefreshOutcome {
                version: CONTRACT_VERSION,
                dataset_id: "sales".to_owned(),
                ok: true,
                rows: 42,
                duration_ms: 1_250,
                data_source: Some("warehouse".to_owned()),
                error: None,
            },
            ReportRefreshOutcome {
                version: CONTRACT_VERSION,
                dataset_id: "returns".to_owned(),
                ok: false,
                rows: 0,
                duration_ms: 0,
                data_source: None,
                error: Some("unknown data source".to_owned()),
            },
        ],
    };
    let wire = serde_json::to_value(&refreshed).expect("refresh result serializes");
    assert_eq!(wire["outcomes"][1]["error"], json!("unknown data source"));
    assert_eq!(wire["outcomes"][0]["error"], json!(null));
    assert_eq!(wire["outcomes"][0]["duration_ms"], json!(1_250));
    assert_eq!(wire["report"]["kind"], json!("workbook"));
    assert_eq!(wire["report"]["hidden"], json!(true));
    let restored: ReportRefreshResult =
        serde_json::from_value(wire).expect("refresh result deserializes");
    assert_eq!(restored, refreshed);
}

#[test]
fn a_report_thumbnail_carries_its_image_as_base64_and_names_its_format() {
    use xfalcon_contracts::reports::{ReportId, ReportThumbnail, ReportThumbnailFormat};

    let thumbnail = ReportThumbnail {
        version: CONTRACT_VERSION,
        id: ReportId::from("report-1"),
        format: ReportThumbnailFormat::Webp,
        data_base64: "UklGRg==".to_owned(),
    };
    let wire = serde_json::to_value(&thumbnail).expect("thumbnail serializes");
    assert_eq!(
        wire,
        json!({ "version": 1, "id": "report-1", "format": "webp", "data_base64": "UklGRg==" })
    );
    let restored: ReportThumbnail = serde_json::from_value(wire).expect("thumbnail round-trips");
    assert_eq!(restored, thumbnail);
    assert!(serde_json::from_value::<ReportThumbnail>(
        json!({ "version": 1, "id": "report-1", "format": "gif", "data_base64": "" })
    )
    .is_err());
}

#[test]
fn a_capture_document_carries_only_a_preview_id_and_its_nonce() {
    use xfalcon_contracts::reports::{ReportCaptureDocument, ReportId};

    let wire = serde_json::to_value(ReportCaptureDocument {
        version: CONTRACT_VERSION,
        id: ReportId::from("report-1"),
        document_id: "c".repeat(64),
        nonce: "n".repeat(32),
    })
    .expect("capture document serializes");
    let mut keys = object_keys(&wire);
    keys.sort_unstable();
    assert_eq!(keys, vec!["document_id", "id", "nonce", "version"]);
}

#[test]
fn claude_effort_is_the_closed_set_of_claude_flag_values_and_defaults_to_high() {
    assert_eq!(ClaudeEffort::default(), ClaudeEffort::High);
    assert!(
        ClaudeEffort::decl().contains(r#""low" | "medium" | "high" | "xhigh" | "max""#),
        "the TypeScript type must be the five levels by their flag values"
    );

    for (index, level) in ClaudeEffort::ALL.iter().enumerate() {
        assert!(
            !ClaudeEffort::ALL[index + 1..].contains(level),
            "{level:?} is listed twice"
        );
        let wire = serde_json::to_value(level).expect("effort serializes");
        assert_eq!(wire, json!(level.flag()));
        assert_eq!(
            serde_json::from_value::<ClaudeEffort>(wire).expect("effort deserializes"),
            *level
        );
    }

    // `ultracode` would switch on Claude Code's own workflow mode, and a Rust
    // variant name is not a wire value, so neither crosses as an effort.
    for rejected in [r#""ultracode""#, r#""High""#] {
        serde_json::from_str::<ClaudeEffort>(rejected)
            .expect_err("a value outside the five flag values must be refused");
    }

    serde_json::from_value::<ClaudeEffortInput>(json!({ "effort": "max" }))
        .expect("a level is a valid input");
    serde_json::from_value::<ClaudeEffortInput>(json!({ "effort": "max", "model": "opus" }))
        .expect_err("effort input must reject unknown fields");
}
