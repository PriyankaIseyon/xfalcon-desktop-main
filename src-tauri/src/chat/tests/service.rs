use std::sync::{Arc, Once};

use crate::auth::AuthSessionStore;
use crate::chat::domain::MAX_MESSAGE_TEXT_BYTES;
use crate::chat::service::{ChatService, MAX_PROMPT_LEN};
use crate::credentials::CredentialRuntime;
use crate::persistence::InMemorySecretStore;
use crate::security::account_cleanup::CurrentAccountScope;
use xfalcon_contracts::chat::ChatErrorCode;

/// `ChatService::authenticated` checks `AuthConfig::from_env()` *before* the
/// session, so on a machine with no auth environment every call short-circuits
/// to `NotConfigured` ("Auth is not configured on this device.") and the
/// sign-in branch below is unreachable. That ordering is correct in
/// production — a deployment misconfiguration should not be reported to the
/// user as "please sign in" — so the fixture supplies configuration rather
/// than the service being reordered to suit a test.
///
/// Set once per process, via `Once`, because the environment is global and
/// these tests run concurrently with the rest of the suite. No test asserts
/// the `NotConfigured` branch, so populating these cannot mask another
/// expectation.
fn configure_auth_environment() {
    static CONFIGURED: Once = Once::new();
    CONFIGURED.call_once(|| {
        std::env::set_var("XFALCON_DA_BASE_URL", "https://da.test.invalid");
        std::env::set_var("XFALCON_CASDOOR_ISSUER", "https://auth.test.invalid");
        std::env::set_var("XFALCON_CASDOOR_CLIENT_ID", "test-client");
        std::env::set_var("XFALCON_CASDOOR_ORGANIZATION", "test-org");
    });
}

/// The composer's ceiling and the domain's ceiling must be one number.
///
/// They were two once. Raising the prompt ceiling alone let an over-length
/// prompt pass `message_send`'s friendly check and fail in `ensure_text_size`
/// instead -- after `authenticated_fresh`, `sync_service` and
/// `reconcile_conversation` had run, so the person paid three network round
/// trips for a refusal the composer could have given immediately. `MAX_PROMPT_LEN`
/// is defined as `MAX_MESSAGE_TEXT_BYTES` to make that impossible; this test
/// fails if either is ever given its own literal again.
#[test]
fn the_prompt_ceiling_is_the_domains_message_ceiling() {
    assert_eq!(MAX_PROMPT_LEN, MAX_MESSAGE_TEXT_BYTES);
}

fn service() -> ChatService {
    service_for(
        Arc::new(AuthSessionStore::new()),
        Arc::new(CurrentAccountScope::default()),
    )
}

fn service_for(sessions: Arc<AuthSessionStore>, accounts: Arc<CurrentAccountScope>) -> ChatService {
    configure_auth_environment();
    let secrets = Arc::new(InMemorySecretStore::default());
    ChatService::new(
        sessions.clone(),
        accounts.clone(),
        std::env::temp_dir(),
        secrets.clone(),
        Arc::new(CredentialRuntime::new(
            sessions,
            Arc::new(InMemorySecretStore::default()),
        )),
        Arc::new(crate::clients::da::model_catalog::ModelCatalogCache::default()),
        Arc::new(crate::projects::ProjectsService::new(
            std::env::temp_dir(),
            accounts.clone(),
            secrets.clone(),
        )),
        Arc::new(crate::approvals::ApprovalsService::new(
            std::env::temp_dir(),
            accounts,
            secrets,
        )),
    )
}

#[tokio::test]
async fn workspace_bootstrap_requires_authentication() {
    let error = service()
        .workspace_bootstrap(None)
        .await
        .expect_err("expected auth error");
    // The code, not the prose: the sentence is written for a reader and may be
    // reworded, while the code is the contract the frontend branches on.
    assert_eq!(error.code(), ChatErrorCode::AuthRequired);
    assert_eq!(error.to_string(), "Sign in to use chat.");
}

/// Signed in as `tenant-1:user-1` an hour past expiry, with no refresh token:
/// a session no server will ever accept again.
fn dead_session_service() -> ChatService {
    let sessions = Arc::new(AuthSessionStore::new());
    sessions.set_authenticated(
        xfalcon_contracts::auth::SessionProjection::authenticated(
            xfalcon_contracts::auth::IdentityProjection::new("user-1", "tenant-1", "User", None),
            chrono::Utc::now().timestamp_millis() - 3_600_000,
        ),
        "access",
        None,
    );
    let accounts = Arc::new(CurrentAccountScope::default());
    accounts
        .bind_native("tenant-1:user-1")
        .expect("bind account");
    service_for(sessions, accounts)
}

/// Search reads only this device's sealed rows, so no server would refuse a
/// dead session on its behalf. It is refused before any row is opened, with
/// the code the data plane's own refusal of an expired bearer maps to.
#[tokio::test]
async fn conversation_search_refuses_a_dead_session() {
    let error = dead_session_service()
        .conversation_search("hello".to_owned())
        .await
        .expect_err("a dead session cannot read the local rows");

    assert_eq!(error.code(), ChatErrorCode::SessionExpired);
}

#[tokio::test]
async fn conversation_preview_refuses_a_dead_session() {
    let error = dead_session_service()
        .conversation_preview(xfalcon_contracts::ConversationId::from(
            uuid::Uuid::new_v4().to_string(),
        ))
        .await
        .expect_err("a dead session cannot read the local rows");

    assert_eq!(error.code(), ChatErrorCode::SessionExpired);
}

/// The dead-session check comes after the existing ones, so a signed-out
/// device keeps the code it has always answered with.
#[tokio::test]
async fn conversation_search_while_signed_out_still_asks_for_sign_in() {
    let error = service()
        .conversation_search("hello".to_owned())
        .await
        .expect_err("signed out");

    assert_eq!(error.code(), ChatErrorCode::AuthRequired);
}

#[tokio::test]
async fn message_send_rejects_empty_prompt() {
    let result = service()
        .message_send(None, "   ".to_owned(), None, Vec::new(), None)
        .await;
    assert_eq!(result.message, "Enter a message to send.");
}

#[tokio::test]
async fn message_send_accepts_an_empty_prompt_when_attachments_are_present() {
    let result = service()
        .message_send(
            None,
            "   ".to_owned(),
            None,
            vec![uuid::Uuid::new_v4()],
            None,
        )
        .await;
    assert_ne!(result.message, "Enter a message to send.");
}

#[tokio::test]
async fn message_send_rejects_while_send_in_flight() {
    let svc = service();
    svc.set_send_in_flight_for_tests(true).await;
    let result = svc
        .message_send(None, "hello".to_owned(), None, Vec::new(), None)
        .await;
    assert_eq!(
        result.message,
        "Wait for the current message to finish sending."
    );
}

#[tokio::test]
async fn message_send_rejects_when_unauthenticated() {
    let result = service()
        .message_send(None, "hello".to_owned(), None, Vec::new(), None)
        .await;
    assert!(result.message.contains("Sign in"));
}

#[tokio::test]
async fn message_send_rejects_overlong_prompt() {
    let prompt = "x".repeat(crate::chat::service::MAX_PROMPT_LEN + 1);
    let result = service()
        .message_send(None, prompt, None, Vec::new(), None)
        .await;
    assert_eq!(result.message, "Message is too long.");
}

/// This device is the only thing that executes a run — DA stores runs and calls
/// no provider — so a send it cannot execute must be refused before a
/// conversation, message, or run row is written. See
/// `ChatService::inference_credential`.
mod executability {
    use super::service;
    use crate::clients::da::chat_api::{select_model, ModelExecutionMode, ModelSelection};
    use crate::credentials::service::CredentialServiceError;
    use crate::security::account_cleanup::ValidatedAccountId;

    fn account() -> ValidatedAccountId {
        ValidatedAccountId::parse("tenant:user").expect("account")
    }

    fn selection(provider: &str) -> ModelSelection {
        ModelSelection {
            model_id: "vendor/model".to_owned(),
            provider: provider.to_owned(),
            context_length: 1_000_000,
            max_output_tokens: 64_000,
            execution_mode: ModelExecutionMode::BoundedContext,
            custom: None,
        }
    }

    #[tokio::test]
    async fn a_provider_this_build_cannot_run_is_named_in_the_refusal() {
        let error = service()
            .inference_credential(&account(), &selection("mistral"))
            .await
            .expect_err("no adapter serves `mistral`");

        assert!(
            error.contains("mistral"),
            "the refusal must name the provider it cannot run: {error}"
        );
    }

    #[tokio::test]
    async fn a_supported_provider_requires_authentication_before_key_lookup() {
        // This fixture is deliberately signed out. The old catch-all mapping
        // disguised that state as a missing key, so this test used to pass for
        // the wrong reason without ever reaching a credential catalog.
        let error = service()
            .inference_credential(&account(), &selection("openrouter"))
            .await
            .expect_err("a signed-out service cannot look up credentials");

        assert!(
            error.contains("Sign in"),
            "the refusal must name the required action: {error}"
        );
    }

    #[tokio::test]
    async fn an_empty_catalog_has_no_hidden_fallback_model() {
        assert_eq!(
            select_model(&[]),
            Err(crate::clients::da::chat_api::DaChatApiError::NoModel)
        );
    }

    #[test]
    fn credential_failures_keep_the_action_that_can_resolve_them() {
        use crate::chat::service::inference_credential_error;

        assert_eq!(
            inference_credential_error(CredentialServiceError::Unauthorized),
            "Your session expired. Sign in again."
        );
        assert!(
            inference_credential_error(CredentialServiceError::RateLimited)
                .contains("rate limited")
        );
        assert!(
            inference_credential_error(CredentialServiceError::ServiceUnavailable)
                .contains("unavailable")
        );
        assert_eq!(
            inference_credential_error(CredentialServiceError::NoUsableCredential),
            "No usable key for this model's provider. Add or re-enter one in Settings → Providers."
        );
    }
}

/// A plan model (`claude_code/...`, `chatgpt_plan/...`) resolves its credential
/// and adapter through `PlanProviders` only. See `ChatService::inference_credential`
/// and `adapter_for_selected_model`.
mod plan_models {
    use std::sync::Arc;

    use super::{catalog_row, service};
    use crate::chat::service::{adapter_for_selected_model, is_plan_provider, model_catalog_view};
    use crate::clients::da::chat_api::{ModelExecutionMode, ModelSelection, HAIKU_MODEL_ID};
    use crate::clients::da::generated::ModelEntity;
    use crate::persistence::InMemorySecretStore;
    use crate::plan_providers::chat_effort::ChatEffortStore;
    use crate::plan_providers::settings::PlanSettingsStore;
    use crate::plan_providers::PlanProviders;
    use crate::providers::ProviderError;
    use crate::security::account_cleanup::ValidatedAccountId;
    use xfalcon_contracts::credentials::{ModelId, ProviderId};
    use xfalcon_contracts::plan_providers::ClaudeEffort;
    use xfalcon_contracts::ConversationId;

    fn account() -> ValidatedAccountId {
        ValidatedAccountId::parse("tenant:user").expect("account")
    }

    fn selection(provider: &str, model_id: &str) -> ModelSelection {
        ModelSelection {
            model_id: model_id.to_owned(),
            provider: provider.to_owned(),
            context_length: 1_000_000,
            max_output_tokens: 64_000,
            execution_mode: ModelExecutionMode::BoundedContext,
            custom: None,
        }
    }

    /// Claude is off by default and no ChatGPT sign-in exists, so neither path
    /// starts a process or reaches the network.
    fn plan_providers() -> (tempfile::TempDir, Arc<PlanProviders>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let plan = Arc::new(PlanProviders::new(
            dir.path().to_path_buf(),
            Arc::new(InMemorySecretStore::default()),
        ));
        (dir, plan)
    }

    #[tokio::test]
    async fn a_plan_model_is_refused_when_no_plan_providers_are_wired() {
        let error = service()
            .inference_credential(&account(), &selection("claude_code", "claude_code/haiku"))
            .await
            .expect_err("no plan providers are wired");

        assert_eq!(error, "Plan models are not available on this device.");
    }

    #[tokio::test]
    async fn a_claude_model_is_refused_while_claude_is_off() {
        let (_dir, plan) = plan_providers();
        let chat = service();
        chat.set_plan_providers(plan);

        let error = chat
            .inference_credential(&account(), &selection("claude_code", "claude_code/haiku"))
            .await
            .expect_err("Claude is off by default");

        assert!(error.contains("turned off"), "{error}");
    }

    #[tokio::test]
    async fn a_chatgpt_model_is_refused_without_a_sign_in() {
        let (_dir, plan) = plan_providers();
        let chat = service();
        chat.set_plan_providers(plan);

        let error = chat
            .inference_credential(&account(), &selection("chatgpt_plan", "chatgpt_plan/gpt-5"))
            .await
            .expect_err("no ChatGPT sign-in exists");

        assert!(error.contains("Sign in to ChatGPT"), "{error}");
    }

    /// The title upgrade and the follow-up suggestions are calls the person did
    /// not ask for. Only the plan providers are exempt: they are the ones whose
    /// calls spend a plan's quota. The follow-up command and the title spawn both
    /// read this predicate before they build an adapter.
    #[test]
    fn only_the_plan_providers_skip_the_title_and_follow_up_calls() {
        assert!(is_plan_provider(ProviderId::ClaudeCode));
        assert!(is_plan_provider(ProviderId::ChatgptPlan));
        for provider in [
            ProviderId::OpenAi,
            ProviderId::Anthropic,
            ProviderId::Google,
            ProviderId::OpenRouter,
            ProviderId::CustomOpenAi,
        ] {
            assert!(!is_plan_provider(provider), "{provider:?}");
        }
    }

    #[test]
    fn plan_rows_are_listed_under_their_own_provider() {
        let plan_row = |model_id: &str, provider: &str| ModelEntity {
            provider: provider.to_owned(),
            provider_parent: provider.to_owned(),
            ..catalog_row(model_id, "Plan model")
        };
        let rows = [
            catalog_row(HAIKU_MODEL_ID, "Haiku"),
            plan_row("claude_code/haiku", "claude_code"),
            plan_row("chatgpt_plan/gpt-5", "chatgpt_plan"),
        ];
        let selected = selection("openrouter", HAIKU_MODEL_ID);

        let view = model_catalog_view(&rows, &selected, false);
        let provider_of = |model_id: &str| {
            view.models
                .iter()
                .find(|model| model.id == ModelId::from(model_id))
                .map(|model| model.provider)
        };

        assert_eq!(provider_of(HAIKU_MODEL_ID), Some(ProviderId::OpenRouter));
        assert_eq!(
            provider_of("claude_code/haiku"),
            Some(ProviderId::ClaudeCode)
        );
        assert_eq!(
            provider_of("chatgpt_plan/gpt-5"),
            Some(ProviderId::ChatgptPlan)
        );
    }

    #[test]
    fn a_plan_model_has_no_adapter_without_plan_providers() {
        let error = adapter_for_selected_model(
            ProviderId::ClaudeCode,
            None,
            &std::env::temp_dir(),
            &account(),
            None,
            None,
        )
        .err();

        assert_eq!(error, Some(ProviderError::RuntimeUnavailable));
    }

    #[test]
    fn a_plan_model_with_claude_off_has_no_adapter() {
        let (dir, plan) = plan_providers();

        let error = adapter_for_selected_model(
            ProviderId::ClaudeCode,
            None,
            dir.path(),
            &account(),
            Some(&plan),
            None,
        )
        .err();

        assert_eq!(error, Some(ProviderError::RuntimeUnavailable));
    }

    /// A Claude turn passes its conversation through `adapter_for_selected_model`,
    /// so the conversation keeps the level it was first built with.
    #[test]
    fn a_claude_turn_locks_its_conversation_level() {
        let (dir, plan) = plan_providers();
        let settings = PlanSettingsStore::new(dir.path());
        settings.set_claude_enabled(true).expect("settings");
        settings
            .set_claude_effort(ClaudeEffort::Low)
            .expect("settings");
        let conversation_id = ConversationId::from("00000000-0000-0000-0000-000000000001");

        adapter_for_selected_model(
            ProviderId::ClaudeCode,
            None,
            dir.path(),
            &account(),
            Some(&plan),
            Some(&conversation_id),
        )
        .expect("adapter");

        settings
            .set_claude_effort(ClaudeEffort::Max)
            .expect("settings");
        assert_eq!(
            ChatEffortStore::new(dir.path(), &account())
                .lock_or_get(conversation_id.as_str(), ClaudeEffort::High),
            ClaudeEffort::Low
        );
    }

    #[test]
    fn an_ordinary_model_keeps_its_vendor_adapter() {
        let adapter = adapter_for_selected_model(
            ProviderId::OpenRouter,
            None,
            &std::env::temp_dir(),
            &account(),
            None,
            None,
        )
        .expect("OpenRouter has a vendor adapter");

        assert_eq!(adapter.provider_id(), ProviderId::OpenRouter);
    }
}

#[tokio::test]
async fn run_cancel_rejects_unknown_run() {
    let error = service()
        .run_cancel("00000000-0000-0000-0000-000000000000".to_owned(), None)
        .await
        .expect_err("unknown run");
    assert!(error.to_string().contains("No active run"));
}

#[test]
fn run_journal_cleanup_retains_recoverable_states() {
    use crate::chat::service::should_remove_run_journal;
    use xfalcon_contracts::RunState;

    assert!(!should_remove_run_journal(&RunState::Queued));
    assert!(!should_remove_run_journal(&RunState::Running));
    assert!(!should_remove_run_journal(&RunState::Interrupted));
    assert!(should_remove_run_journal(&RunState::Completed));
    assert!(should_remove_run_journal(&RunState::Failed));
    assert!(should_remove_run_journal(&RunState::Cancelled));
}

mod tool_history_replay {
    use uuid::Uuid;

    use crate::agent::context::ContextRole;
    use crate::chat::history::context_from_history;
    use crate::clients::da::generated::{ChatMessage, ChatMessageToolCalls};

    fn row(sequence_no: i64, role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: Uuid::new_v4(),
            tenant_id: "tenant".to_owned(),
            conversation_id: Uuid::new_v4(),
            sequence_no,
            role: role.to_owned(),
            content: content.to_owned(),
            ..Default::default()
        }
    }

    /// An assistant turn as any client writes it: prose (often empty) plus the
    /// OpenAI call pairing in `tool_calls`.
    fn requesting(sequence_no: i64, content: &str, calls: &[(&str, &str)]) -> ChatMessage {
        let mut message = row(sequence_no, "assistant", content);
        message.tool_calls = Some(ChatMessageToolCalls::Array(
            calls
                .iter()
                .map(|(call_id, name)| {
                    serde_json::json!({
                        "id": call_id,
                        "type": "function",
                        "function": {"name": name, "arguments": r#"{"q":"budget"}"#},
                    })
                })
                .collect(),
        ));
        message
    }

    fn answering(sequence_no: i64, call_id: &str, name: &str, content: &str) -> ChatMessage {
        let mut message = row(sequence_no, "tool", content);
        message.tool_call_id = Some(call_id.to_owned());
        message.name = Some(name.to_owned());
        message
    }

    /// The point of the whole feature: a follow-up turn must see what the last
    /// turn looked up, as a well-formed call/result pair — and it must see it
    /// whichever client ran that turn.
    #[test]
    fn a_prior_turns_tool_round_replays_as_a_paired_assistant_turn_and_result() {
        let rows = vec![
            row(1, "user", "how much did we spend?"),
            requesting(2, "", &[("call_a", "project_search")]),
            answering(3, "call_a", "project_search", r#"{"matches":3}"#),
            row(4, "assistant", "spend was 12"),
        ];

        let context = context_from_history(&rows);

        assert_eq!(context.len(), 4, "{context:#?}");
        assert_eq!(context[0].role, ContextRole::User);
        assert_eq!(context[1].role, ContextRole::Assistant);
        assert_eq!(context[1].tool_calls.len(), 1);
        assert_eq!(context[1].tool_calls[0].call_id, "call_a");
        assert_eq!(context[1].tool_calls[0].name, "project_search");
        assert!(context[1].tool_calls[0].arguments.contains("budget"));
        assert_eq!(context[2].role, ContextRole::Tool);
        assert_eq!(
            context[2].responds_to.as_ref().map(|r| r.call_id.as_str()),
            Some("call_a")
        );
        assert!(context[2].content.contains("matches"));
        assert_eq!(context[3].content, "spend was 12");
    }

    /// Rows arrive over the network and a write can fail between the two halves.
    /// A call with no answer is the one shape every provider rejects outright, so
    /// the turn degrades to its prose rather than poisoning the whole request.
    #[test]
    fn an_advertised_call_with_no_answer_is_replayed_as_prose() {
        let rows = vec![
            row(1, "user", "how much did we spend?"),
            requesting(2, "checking", &[("call_a", "project_search")]),
        ];

        let context = context_from_history(&rows);

        assert_eq!(context.len(), 2, "{context:#?}");
        assert_eq!(context[1].content, "checking");
        assert!(context[1].tool_calls.is_empty());
    }

    /// The mirror case: an answer naming no call is equally rejected, and a
    /// calls-only turn with nothing to say leaves nothing behind.
    #[test]
    fn an_answer_naming_no_call_is_dropped_and_leaves_no_empty_turn() {
        let rows = vec![
            row(1, "user", "how much did we spend?"),
            requesting(2, "", &[("call_a", "project_search")]),
            answering(3, "call_zzz", "project_search", r#"{"matches":3}"#),
        ];

        let context = context_from_history(&rows);

        assert_eq!(context.len(), 1, "{context:#?}");
        assert_eq!(context[0].role, ContextRole::User);
    }

    /// Parallel calls in one round replay in the order they were advertised, and
    /// every one of them is answered.
    #[test]
    fn every_call_in_a_parallel_round_is_answered() {
        let rows = vec![
            requesting(1, "", &[("call_a", "project_search"), ("call_b", "read")]),
            answering(2, "call_b", "read", "b"),
            answering(3, "call_a", "project_search", "a"),
        ];

        let context = context_from_history(&rows);

        assert_eq!(context.len(), 3);
        assert_eq!(context[0].tool_calls.len(), 2);
        let answered: Vec<&str> = context
            .iter()
            .filter_map(|m| m.responds_to.as_ref())
            .map(|r| r.call_id.as_str())
            .collect();
        assert_eq!(
            answered,
            vec!["call_a", "call_b"],
            "answers follow the order the calls were advertised, not the order they landed"
        );
    }

    /// `sequence_no` is the only total order over a conversation: parallel calls
    /// in one round share `created_at` to the millisecond.
    #[test]
    fn rows_are_ordered_by_sequence_regardless_of_arrival_order() {
        let rows = vec![
            row(3, "assistant", "third"),
            row(1, "user", "first"),
            row(2, "assistant", "second"),
        ];

        let context = context_from_history(&rows);

        let texts: Vec<&str> = context.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
    }

    /// A conversation rewound to a checkpoint must replay as the user now sees
    /// it, not as it once was.
    #[test]
    fn a_superseded_row_is_not_replayed() {
        let mut superseded = row(2, "assistant", "the retracted answer");
        superseded.superseded_by_checkpoint_id = Some(Uuid::new_v4());
        let rows = vec![row(1, "user", "kept"), superseded];

        let context = context_from_history(&rows);

        assert_eq!(context.len(), 1);
        assert_eq!(context[0].content, "kept");
    }

    /// Two turns cannot both claim one answer — the second would send an answer
    /// the request never asked for.
    #[test]
    fn one_answer_is_spent_by_one_turn() {
        let rows = vec![
            requesting(1, "", &[("call_a", "project_search")]),
            answering(2, "call_a", "project_search", "a"),
            requesting(3, "again", &[("call_a", "project_search")]),
        ];

        let context = context_from_history(&rows);

        assert_eq!(context.len(), 3, "{context:#?}");
        assert_eq!(context[2].content, "again");
        assert!(context[2].tool_calls.is_empty());
    }
}

mod turn_revision_tests {
    use crate::chat::run_journal_store::RunJournalStore;
    use crate::chat::service::{TurnRevision, MAX_PROMPT_LEN};
    use crate::clients::da::generated::TruncateRequest;
    use crate::security::account_cleanup::ValidatedAccountId;
    use std::collections::HashSet;
    use tempfile::tempdir;
    use uuid::Uuid;
    use xfalcon_contracts::RunId;

    #[test]
    fn turn_revision_cutoff_and_user_message_flags() {
        let none = TurnRevision::None;
        assert_eq!(none.cutoff(), None);
        assert!(none.writes_user_message());

        let cut_id = Uuid::new_v4();
        let replace = TurnRevision::Replace { cutoff: cut_id };
        assert_eq!(replace.cutoff(), Some(cut_id));
        assert!(replace.writes_user_message());

        let rerun = TurnRevision::Rerun { cutoff: cut_id };
        assert_eq!(rerun.cutoff(), Some(cut_id));
        assert!(!rerun.writes_user_message());
    }

    #[test]
    fn truncate_request_serializes_from_message_id() {
        let cut_id = Uuid::new_v4();
        let req = TruncateRequest {
            from_message_id: cut_id,
        };
        let serialized = serde_json::to_string(&req).expect("must serialize");
        assert!(serialized.contains(&cut_id.to_string()));
    }

    #[test]
    fn journal_purge_removes_only_reconciled_diff_and_protects_on_none() {
        let directory = tempdir().expect("tempdir");
        let account = ValidatedAccountId::parse("journal-test-account").expect("account");
        let store = RunJournalStore::new(directory.path().to_path_buf(), account);

        let run_1 = Uuid::new_v4();
        let run_2 = Uuid::new_v4();
        let run_3 = Uuid::new_v4();

        let run_id_1 = RunId::from(run_1.to_string());
        let run_id_2 = RunId::from(run_2.to_string());
        let run_id_3 = RunId::from(run_3.to_string());

        // Populate journals on disk
        store
            .append_transition(
                &run_id_1,
                1,
                xfalcon_contracts::RunState::Queued,
                xfalcon_contracts::RunState::Running,
            )
            .expect("append 1");
        store
            .append_transition(
                &run_id_2,
                1,
                xfalcon_contracts::RunState::Queued,
                xfalcon_contracts::RunState::Running,
            )
            .expect("append 2");
        store
            .append_transition(
                &run_id_3,
                1,
                xfalcon_contracts::RunState::Queued,
                xfalcon_contracts::RunState::Running,
            )
            .expect("append 3");

        let mut pre_cut_runs: HashSet<Uuid> = HashSet::new();
        pre_cut_runs.insert(run_1);
        pre_cut_runs.insert(run_2);
        pre_cut_runs.insert(run_3);

        // Case A: Reconcile failed (None). Must NOT purge anything!
        let reconciled_after: Option<HashSet<Uuid>> = None;
        if let Some(post_cut) = reconciled_after {
            for removed_run in pre_cut_runs.difference(&post_cut) {
                store.remove_run(&RunId::from(removed_run.to_string()));
            }
        }
        // All 3 files must still exist and have entries
        assert_eq!(store.load_entries(&run_id_1).expect("load 1").len(), 1);
        assert_eq!(store.load_entries(&run_id_2).expect("load 2").len(), 1);
        assert_eq!(store.load_entries(&run_id_3).expect("load 3").len(), 1);

        // Case B: Reconciled after truncation with run_1 retained
        let mut post_cut_runs: HashSet<Uuid> = HashSet::new();
        post_cut_runs.insert(run_1);

        for removed_run in pre_cut_runs.difference(&post_cut_runs) {
            store.remove_run(&RunId::from(removed_run.to_string()));
        }

        // run_1 is retained, run_2 and run_3 are removed from disk
        assert_eq!(store.load_entries(&run_id_1).expect("load 1").len(), 1);
        assert!(store.load_entries(&run_id_2).expect("empty 2").is_empty());
        assert!(store.load_entries(&run_id_3).expect("empty 3").is_empty());
    }

    #[tokio::test]
    async fn message_send_inner_with_revision_fails_loudly_when_unauthenticated() {
        let svc = super::service();
        let conv_id = xfalcon_contracts::ConversationId::from(Uuid::new_v4().to_string());
        let cutoff = Uuid::new_v4();

        // 1. Replace revision on unauthenticated device
        let replace_result = svc
            .message_send_inner(
                Some(conv_id.clone()),
                "edited prompt text",
                TurnRevision::Replace { cutoff },
                None,
                Vec::new(),
                None,
            )
            .await;
        assert_eq!(
            replace_result.status,
            xfalcon_contracts::chat::ChatSubmitStatus::Failed
        );
        assert!(replace_result.message.contains("Sign in"));

        // 2. Rerun revision on unauthenticated device
        let rerun_result = svc
            .message_send_inner(
                Some(conv_id),
                "original prompt text",
                TurnRevision::Rerun { cutoff },
                None,
                Vec::new(),
                None,
            )
            .await;
        assert_eq!(
            rerun_result.status,
            xfalcon_contracts::chat::ChatSubmitStatus::Failed
        );
        assert!(rerun_result.message.contains("Sign in"));
    }

    #[tokio::test]
    async fn message_edit_validates_inputs_before_accessing_state() {
        let svc = super::service();
        let conv_id = xfalcon_contracts::ConversationId::from(Uuid::new_v4().to_string());

        // Empty text
        let empty_res = svc
            .message_edit(
                None,
                conv_id.clone(),
                Uuid::new_v4().to_string(),
                "   ".to_string(),
                None,
            )
            .await
            .expect("should return ChatSubmitResult");
        assert_eq!(
            empty_res.status,
            xfalcon_contracts::chat::ChatSubmitStatus::Failed
        );
        assert_eq!(empty_res.message, "Enter a message to send.");

        // Overlong text
        let overlong = "a".repeat(MAX_PROMPT_LEN + 1);
        let overlong_res = svc
            .message_edit(
                None,
                conv_id.clone(),
                Uuid::new_v4().to_string(),
                overlong,
                None,
            )
            .await
            .expect("should return ChatSubmitResult");
        assert_eq!(
            overlong_res.status,
            xfalcon_contracts::chat::ChatSubmitStatus::Failed
        );
        assert_eq!(overlong_res.message, "Message is too long.");

        // Invalid message UUID
        let invalid_uuid_res = svc
            .message_edit(
                None,
                conv_id,
                "not-a-valid-uuid".to_string(),
                "valid text".to_string(),
                None,
            )
            .await;
        assert!(invalid_uuid_res.is_err());
        assert_eq!(
            invalid_uuid_res.unwrap_err().to_string(),
            "Message id is invalid."
        );
    }

    #[tokio::test]
    async fn message_regenerate_validates_message_uuid() {
        let svc = super::service();
        let conv_id = xfalcon_contracts::ConversationId::from(Uuid::new_v4().to_string());

        let invalid_uuid_res = svc
            .message_regenerate(None, conv_id, "not-a-valid-uuid".to_string(), None)
            .await;
        assert!(invalid_uuid_res.is_err());
        assert_eq!(
            invalid_uuid_res.unwrap_err().to_string(),
            "Message id is invalid."
        );
    }
}

pub(super) fn catalog_row(
    model_id: &str,
    display_name: &str,
) -> crate::clients::da::generated::ModelEntity {
    crate::clients::da::generated::ModelEntity {
        id: uuid::Uuid::nil(),
        model_id: model_id.to_owned(),
        provider: "openrouter".to_owned(),
        provider_parent: "openrouter".to_owned(),
        provider_sub: None,
        display_name: display_name.to_owned(),
        context_length: 1_050_000,
        max_output_tokens: Some(128_000),
        input_price_per_m: 1.0,
        output_price_per_m: 2.0,
        released_at: None,
        is_chat: true,
        is_enabled: true,
        supports_streaming: true,
        supports_tools: true,
        supports_vision: false,
        metadata: Some(indexmap::IndexMap::new()),
        created_at: None,
        updated_at: None,
        is_active: true,
        tenant_id: "tenant".to_owned(),
        supports_reasoning: false,
        supports_extended_thinking: false,
        reasoning_param_style: "none".to_owned(),
        default_thinking_budget_tokens: None,
        owner_user_id: None,
        base_url: None,
        upstream_model_id: None,
        provider_key_id: None,
    }
}

/// The composer names a conversation's model by finding it in the projected
/// list. A conversation locked to the retired Luna id must still find it, or
/// it reads "Model unavailable" while the model runs.
#[test]
fn a_conversation_locked_to_the_retired_luna_id_lists_its_own_model() {
    use crate::clients::da::chat_api::{
        select_model_by_id, LUNA_MODEL_ID, RETIRED_LUNA_MODEL_ID, SOL_MODEL_ID,
    };
    use xfalcon_contracts::credentials::ModelId;

    let rows = [
        catalog_row(RETIRED_LUNA_MODEL_ID, "Luna"),
        catalog_row(LUNA_MODEL_ID, "Luna"),
        catalog_row(SOL_MODEL_ID, "Sol"),
    ];
    let listed = |view: &xfalcon_contracts::chat::ChatModelCatalogView| {
        view.models
            .iter()
            .map(|model| model.id.clone())
            .collect::<Vec<_>>()
    };

    let locked = select_model_by_id(&rows, Some(RETIRED_LUNA_MODEL_ID)).expect("locked");
    let view = crate::chat::service::model_catalog_view(&rows, &locked, true);
    assert_eq!(view.selected_model_id, ModelId::from(RETIRED_LUNA_MODEL_ID));
    assert_eq!(
        listed(&view),
        vec![
            ModelId::from(LUNA_MODEL_ID),
            ModelId::from(SOL_MODEL_ID),
            ModelId::from(RETIRED_LUNA_MODEL_ID),
        ]
    );

    // A new chat is offered only the product models.
    let offered = select_model_by_id(&rows, Some(SOL_MODEL_ID)).expect("Sol");
    let view = crate::chat::service::model_catalog_view(&rows, &offered, false);
    assert_eq!(
        listed(&view),
        vec![ModelId::from(LUNA_MODEL_ID), ModelId::from(SOL_MODEL_ID)]
    );
}
