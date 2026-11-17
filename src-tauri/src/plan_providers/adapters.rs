//! The two adapters the plan providers hand to the chat path. One keeps the
//! number of running `claude` processes bounded; the other fetches the ChatGPT
//! access token again for every call.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::credentials::ProviderId;

use super::models::PlanModelSource;
use crate::providers::chatgpt_plan::adapter::ChatgptPlanAdapter;
use crate::providers::chatgpt_plan::session::ChatgptPlanSession;
use crate::providers::{ProviderAdapter, ProviderChatRequest, ProviderError, ProviderStreamEvent};

/// Each Claude turn starts one process. Two at a time keeps this device from
/// starting many programs at once, and a chat only ever needs one.
const MAX_CONCURRENT_TURNS: usize = 2;

/// Process-wide, not per adapter: every Claude turn on this device shares the
/// same two permits, however many adapters were built.
static TURN_PERMITS: Semaphore = Semaphore::const_new(MAX_CONCURRENT_TURNS);

/// Takes one of the process-wide permits for a probe run. A probe starts `claude`
/// too, so it counts against the same cap as a turn. The semaphore is never closed,
/// so this waits for a permit and does not fail.
pub(crate) async fn probe_permit() -> SemaphorePermit<'static> {
    TURN_PERMITS
        .acquire()
        .await
        .expect("the turn permits are never closed")
}

/// Holds one of the process-wide turn permits for the whole of each `stream`.
pub(crate) struct LimitedAdapter {
    inner: Arc<dyn ProviderAdapter>,
}

impl LimitedAdapter {
    #[must_use]
    pub(crate) fn new(inner: Arc<dyn ProviderAdapter>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl ProviderAdapter for LimitedAdapter {
    fn provider_id(&self) -> ProviderId {
        self.inner.provider_id()
    }

    async fn stream(
        &self,
        request: &ProviderChatRequest,
        secret: &str,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        // Waiting for a permit must end when the turn is cancelled, or a stopped
        // chat would stay queued behind other turns.
        let _permit = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ProviderError::Cancelled),
            permit = TURN_PERMITS.acquire() => permit.map_err(|_| ProviderError::Unavailable)?,
        };
        self.inner.stream(request, secret, cancel, on_event).await
    }
}

/// The ChatGPT plan route. Each call takes the session's current access token,
/// so a refresh made by an earlier call is in use, and no token is kept here.
pub(crate) struct SessionBackedChatgptAdapter {
    inner: ChatgptPlanAdapter,
    session: Arc<ChatgptPlanSession>,
    models: Arc<PlanModelSource>,
    account: String,
}

impl SessionBackedChatgptAdapter {
    #[must_use]
    pub(crate) fn new(
        session: Arc<ChatgptPlanSession>,
        models: Arc<PlanModelSource>,
        account: String,
    ) -> Self {
        Self {
            inner: ChatgptPlanAdapter::new(),
            session,
            models,
            account,
        }
    }

    /// Sends `id` as the conversation headers of every turn. See
    /// [`ChatgptPlanAdapter::with_conversation`].
    pub(crate) fn with_conversation(mut self, id: Option<&str>) -> Self {
        self.inner = self.inner.with_conversation(id);
        self
    }

    /// OpenAI refused the token, so the sign-in cannot be used. Its sealed record is
    /// removed, which makes the status NeedsSignIn, and the account's ChatGPT rows
    /// are cleared. A failure to remove the record is logged and does not replace
    /// the refusal the turn reports.
    async fn end_refused_sign_in(&self) {
        if let Err(error) = self.session.mark_sign_in_required().await {
            tracing::warn!(error = %error, "could not remove a refused ChatGPT plan sign-in");
        }
        self.models.clear_chatgpt(&self.account);
    }
}

#[async_trait]
impl ProviderAdapter for SessionBackedChatgptAdapter {
    fn provider_id(&self) -> ProviderId {
        ProviderId::ChatgptPlan
    }

    /// The caller's `secret` is ignored: this route has no stored key, and the
    /// token comes from the session for this call only.
    async fn stream(
        &self,
        request: &ProviderChatRequest,
        _secret: &str,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        let now_unix = chrono::Utc::now().timestamp();
        let token = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(ProviderError::Cancelled),
            token = self.session.access_token(now_unix) => token?,
        };
        let outcome = self
            .inner
            .stream(request, token.as_str(), cancel, on_event)
            .await;
        if outcome == Err(ProviderError::PlanSignInRequired) {
            self.end_refused_sign_in().await;
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use reqwest::Url;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::agent::context::{ContextMessage, ContextRole};
    use crate::persistence::InMemorySecretStore;
    use crate::plan_providers::models::PlanModelRow;
    use crate::providers::chatgpt_plan::oauth::{OAuthConfig, PLAN_USAGE_SCOPE};
    use crate::providers::chatgpt_plan::session::SessionState;
    use crate::providers::chatgpt_plan::store::{Meta, PlanStore, Profile, SessionRecord};
    use crate::security::account_cleanup::ValidatedAccountId;

    const ACCOUNT: &str = "person:tenant";
    const CLIENT: &str = "oaiapp_adapter_client";
    const SUBJECT: &str = "user-adapter";
    const CONVERSATION: &str = "3f1c2b8e-6d4a-4f0e-9b7a-2c5d8e1f0a9b";

    fn request() -> ProviderChatRequest {
        ProviderChatRequest {
            model_id: "chatgpt_plan/gpt-6-luna".to_owned(),
            messages: vec![ContextMessage::new(ContextRole::User, "hi")],
            max_output_tokens: None,
            allow_reasoning: false,
            tools: Vec::new(),
            openrouter: None,
        }
    }

    fn now_unix() -> i64 {
        chrono::Utc::now().timestamp()
    }

    fn completed_stream() -> String {
        let payload = json!({
            "type": "response.completed",
            "response": {"usage": {"input_tokens": 1, "output_tokens": 1}},
        });
        format!("data: {payload}\n\n")
    }

    fn session_record(access_token: &str) -> SessionRecord {
        SessionRecord {
            client_id: CLIENT.to_owned(),
            subject: SUBJECT.to_owned(),
            email: None,
            id_token: None,
            access_token: access_token.to_owned(),
            refresh_token: Some("refresh-adapter".to_owned()),
            expires_at: now_unix() + 3_600,
            earliest_refresh_at: None,
            scopes: vec![PLAN_USAGE_SCOPE.to_owned()],
            saved_at: now_unix(),
        }
    }

    fn plan_store(root: &Path, secrets: &Arc<InMemorySecretStore>) -> PlanStore {
        let account = ValidatedAccountId::parse(ACCOUNT).expect("account");
        PlanStore::new(root, &account, secrets.clone())
    }

    /// A session whose one sign-in is on disk, with its endpoints on `server`.
    fn signed_in_session(
        root: &Path,
        secrets: &Arc<InMemorySecretStore>,
        server: &MockServer,
    ) -> Arc<ChatgptPlanSession> {
        let store = plan_store(root, secrets);
        store
            .save_session(&session_record("access-1"))
            .expect("session");
        store
            .save_meta(&Meta {
                host_id: None,
                active_client_id: Some(CLIENT.to_owned()),
                welcome_acknowledged: true,
                profiles: vec![Profile {
                    client_id: CLIENT.to_owned(),
                    subject: SUBJECT.to_owned(),
                    email: None,
                }],
            })
            .expect("meta");
        let issuer = Url::parse(&server.uri()).expect("issuer");
        let models = Url::parse(&format!("{}/v1/models", server.uri())).expect("models");
        Arc::new(
            ChatgptPlanSession::new(store, OAuthConfig::for_test(issuer), models).expect("session"),
        )
    }

    fn adapter_at(
        server: &MockServer,
        session: Arc<ChatgptPlanSession>,
    ) -> SessionBackedChatgptAdapter {
        let endpoint = Url::parse(&format!("{}/v1/responses", server.uri())).expect("url");
        SessionBackedChatgptAdapter {
            inner: ChatgptPlanAdapter::with_endpoint(endpoint),
            session,
            models: Arc::new(PlanModelSource::new()),
            account: ACCOUNT.to_owned(),
        }
    }

    /// Records how many streams are inside the wrapped adapter at once.
    struct OverlapRecorder {
        running: AtomicUsize,
        peak: AtomicUsize,
    }

    #[async_trait]
    impl ProviderAdapter for OverlapRecorder {
        fn provider_id(&self) -> ProviderId {
            ProviderId::ClaudeCode
        }

        async fn stream(
            &self,
            _request: &ProviderChatRequest,
            _secret: &str,
            _cancel: &CancellationToken,
            _on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
        ) -> Result<(), ProviderError> {
            let now_running = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now_running, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(50)).await;
            self.running.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn no_more_than_two_claude_turns_run_at_once() {
        let recorder = Arc::new(OverlapRecorder {
            running: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let limited = LimitedAdapter::new(recorder.clone());
        let request = request();
        let cancel = CancellationToken::new();

        let turns = (0..5).map(|_| {
            let limited = &limited;
            let request = &request;
            let cancel = &cancel;
            async move {
                let mut sink = |_: ProviderStreamEvent| {};
                limited.stream(request, "", cancel, &mut sink).await
            }
        });
        let outcomes = futures_util::future::join_all(turns).await;

        assert!(outcomes.iter().all(|outcome| outcome.is_ok()));
        assert_eq!(recorder.peak.load(Ordering::SeqCst), MAX_CONCURRENT_TURNS);
        assert_eq!(recorder.running.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_cancelled_turn_never_reaches_the_wrapped_adapter() {
        let recorder = Arc::new(OverlapRecorder {
            running: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let limited = LimitedAdapter::new(recorder.clone());
        let cancel = CancellationToken::new();
        cancel.cancel();

        let mut sink = |_: ProviderStreamEvent| {};
        let outcome = limited.stream(&request(), "", &cancel, &mut sink).await;

        assert_eq!(outcome, Err(ProviderError::Cancelled));
        assert_eq!(recorder.peak.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn every_call_takes_the_sessions_current_token() {
        let server = MockServer::start().await;
        for token in ["access-1", "access-2"] {
            Mock::given(method("POST"))
                .and(path("/v1/responses"))
                .and(header("authorization", format!("Bearer {token}")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_raw(completed_stream(), "text/event-stream"),
                )
                .expect(1)
                .mount(&server)
                .await;
        }
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let session = signed_in_session(root.path(), &secrets, &server);
        let adapter = adapter_at(&server, Arc::clone(&session));
        let cancel = CancellationToken::new();

        let mut sink = |_: ProviderStreamEvent| {};
        let first = adapter.stream(&request(), "", &cancel, &mut sink).await;
        plan_store(root.path(), &secrets)
            .save_session(&session_record("access-2"))
            .expect("rotated session");
        let second = adapter.stream(&request(), "", &cancel, &mut sink).await;

        assert_eq!(first, Ok(()));
        assert_eq!(second, Ok(()));
    }

    #[tokio::test]
    async fn a_session_backed_turn_sends_the_conversation_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(header("session-id", CONVERSATION))
            .and(header("thread-id", CONVERSATION))
            .and(header("x-client-request-id", CONVERSATION))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(completed_stream(), "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let session = signed_in_session(root.path(), &secrets, &server);
        let adapter = adapter_at(&server, session).with_conversation(Some(CONVERSATION));

        let mut sink = |_: ProviderStreamEvent| {};
        let outcome = adapter
            .stream(&request(), "", &CancellationToken::new(), &mut sink)
            .await;

        assert_eq!(outcome, Ok(()));
        server.verify().await;
    }

    #[tokio::test]
    async fn without_a_sign_in_no_request_is_sent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let endpoint = Url::parse(&format!("{}/v1/responses", server.uri())).expect("url");
        let session = Arc::new(
            ChatgptPlanSession::new(
                plan_store(root.path(), &secrets),
                OAuthConfig::for_test(Url::parse(&server.uri()).expect("issuer")),
                Url::parse(&format!("{}/v1/models", server.uri())).expect("models"),
            )
            .expect("session"),
        );
        let adapter = SessionBackedChatgptAdapter {
            inner: ChatgptPlanAdapter::with_endpoint(endpoint),
            session,
            models: Arc::new(PlanModelSource::new()),
            account: ACCOUNT.to_owned(),
        };

        let mut sink = |_: ProviderStreamEvent| {};
        let outcome = adapter
            .stream(&request(), "", &CancellationToken::new(), &mut sink)
            .await;

        assert_eq!(outcome, Err(ProviderError::PlanSignInRequired));
    }

    /// OpenAI answers 401 to the token. The turn reports sign-in required, the
    /// sealed sign-in is removed so the status reads NeedsSignIn, and the account's
    /// ChatGPT rows are cleared.
    #[tokio::test]
    async fn a_refused_token_ends_the_sign_in_and_clears_the_account_rows() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let session = signed_in_session(root.path(), &secrets, &server);
        let models = Arc::new(PlanModelSource::new());
        models.set_chatgpt(
            ACCOUNT,
            vec![PlanModelRow {
                provider: "chatgpt_plan",
                slug: "gpt-6-luna".to_owned(),
                display_name: "Luna".to_owned(),
                context_length: 1_050_000,
                max_output_tokens: 128_000,
                supports_vision: true,
            }],
        );
        let endpoint = Url::parse(&format!("{}/v1/responses", server.uri())).expect("url");
        let adapter = SessionBackedChatgptAdapter {
            inner: ChatgptPlanAdapter::with_endpoint(endpoint),
            session: Arc::clone(&session),
            models: Arc::clone(&models),
            account: ACCOUNT.to_owned(),
        };

        let mut sink = |_: ProviderStreamEvent| {};
        let outcome = adapter
            .stream(&request(), "", &CancellationToken::new(), &mut sink)
            .await;

        assert_eq!(outcome, Err(ProviderError::PlanSignInRequired));
        assert_eq!(session.status().state, SessionState::NeedsSignIn);
        assert_eq!(models.chatgpt_count(ACCOUNT), 0);
        server.verify().await;
    }

    #[test]
    fn the_session_backed_adapter_reports_the_chatgpt_plan_id() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let session = Arc::new(
            ChatgptPlanSession::production(plan_store(root.path(), &secrets))
                .expect("production session"),
        );

        assert_eq!(
            SessionBackedChatgptAdapter::new(
                session,
                Arc::new(PlanModelSource::new()),
                ACCOUNT.to_owned()
            )
            .provider_id(),
            ProviderId::ChatgptPlan
        );
        assert_eq!(
            LimitedAdapter::new(Arc::new(OverlapRecorder {
                running: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
            }))
            .provider_id(),
            ProviderId::ClaudeCode
        );
    }
}
