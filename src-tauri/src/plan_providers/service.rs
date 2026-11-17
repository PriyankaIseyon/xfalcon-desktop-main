//! The one object the plan providers are reached through. It owns the device
//! switches, the readiness probe for `claude`, each account's ChatGPT sign-in,
//! the model rows those providers offer, and the credential and adapter for
//! every turn on them.

use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::credentials::ProviderId;
use xfalcon_contracts::plan_providers::{
    ChatgptPlanState, ChatgptPlanStatusView, ClaudeEffort, ClaudeRuntimeState,
    ClaudeRuntimeStatusView,
};
use xfalcon_contracts::CONTRACT_VERSION;
use zeroize::Zeroizing;

use super::adapters::{probe_permit, LimitedAdapter, SessionBackedChatgptAdapter};
use super::chat_effort::ChatEffortStore;
use super::models::{chatgpt_rows, claude_rows, PlanModelSource};
use super::settings::PlanSettingsStore;
use crate::credentials::service::ResolvedCredential;
use crate::persistence::SecretStore;
use crate::providers::chatgpt_plan::oauth::RevokeOutcome;
use crate::providers::chatgpt_plan::session::{
    BrowserOpener, ChatgptPlanSession, SessionError, SessionState, SessionStatus,
};
use crate::providers::chatgpt_plan::store::PlanStore;
use crate::providers::claude_runtime::adapter::{
    probe as probe_claude_runtime, ClaudeRuntimeAdapter, ProbeResult, ProbeState,
};
use crate::providers::claude_runtime::locate::MIN_CLAUDE_VERSION;
use crate::providers::{with_retry, ProviderAdapter, ProviderError};
use crate::security::account_cleanup::ValidatedAccountId;

pub(crate) const WORK_ROOT_DIR: &str = "claude-turns";
/// The prefix `claude_runtime` gives each turn directory.
const TURN_DIR_PREFIX: &str = "turn-";
/// How long a probe answer is reused before the probe runs again.
pub(super) const PROBE_TTL: Duration = Duration::from_secs(30);
/// How long a failed check is reused. It is short, so a transient failure is checked
/// again soon.
const CHECK_FAILED_PROBE_TTL: Duration = Duration::from_secs(5);
/// How long Claude's rows stay offered after the probe that last answered `Ready`.
/// `models.rs` reads it. A stale probe does not hide them: a read renews the probe in
/// the background and keeps the rows until the new answer is applied.
pub(super) const ROWS_STALE_AFTER: Duration = Duration::from_secs(600);
/// The most a send waits to renew the Claude rows. The workspace refresh uses the
/// same limit.
const RENEW_LIMIT: Duration = Duration::from_secs(4);
const CLAUDE_SIGN_IN_COMMAND: &str = "claude auth login";
const CHATGPT_USAGE_URL: &str = "https://chatgpt.com/settings/usage";

const CLAUDE_OFF_TEXT: &str =
    "Claude is turned off on this device. Turn it on in Settings → Models & connections.";
const CHATGPT_SIGN_IN_TEXT: &str =
    "Sign in to ChatGPT in Settings → Models & connections to use your plan.";
const CHATGPT_CHECK_FAILED_TEXT: &str =
    "Could not check your ChatGPT sign-in on this device. Try again.";
const NOT_A_PLAN_TEXT: &str = "This provider does not use a plan sign-in.";
const CHATGPT_UNAVAILABLE_TEXT: &str = "ChatGPT sign-in is not available on this device.";
const SIGN_OUT_UNCONFIRMED_TEXT: &str = "Signed out here, but OpenAI did not confirm it. You can also disconnect xFalcon in your ChatGPT settings.";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum PlanProviderError {
    #[error("the plan settings could not be saved")]
    Settings,
}

type ProbeFuture = Pin<Box<dyn Future<Output = ProbeResult> + Send>>;
/// Asks `claude` for its state. A test answers it without starting a process.
type Probe = Arc<dyn Fn() -> ProbeFuture + Send + Sync>;

pub(crate) struct PlanProviders {
    app_local_data: PathBuf,
    settings: PlanSettingsStore,
    models: Arc<PlanModelSource>,
    secrets: Arc<dyn SecretStore>,
    work_root: PathBuf,
    claude_probe: Probe,
    /// The last probe answer and when it finished. The probe task writes it, so it is
    /// shared with that task.
    probe_cache: Arc<Mutex<Option<(Instant, ProbeResult)>>>,
    /// Serializes the probes. A caller that waits reads the answer of the probe in
    /// flight, and does not start another. The probe task holds it until the answer
    /// is cached, so it is shared with that task.
    probe_lock: Arc<AsyncMutex<()>>,
    sessions: Mutex<HashMap<String, Arc<ChatgptPlanSession>>>,
    sign_ins: Mutex<HashMap<String, CancellationToken>>,
    /// Serializes model refreshes, so two of them cannot write the rows in
    /// opposite orders.
    refresh_lock: AsyncMutex<()>,
}

impl PlanProviders {
    /// Also removes the turn directories a crash left behind. Each holds a
    /// prompt and a loopback token, so a stale one should not stay on disk.
    pub(crate) fn new(app_local_data: PathBuf, secrets: Arc<dyn SecretStore>) -> Self {
        let work_root = app_local_data.join(WORK_ROOT_DIR);
        sweep_stale_turns(&work_root);
        Self::assemble(
            app_local_data,
            secrets,
            work_root,
            Arc::new(|| -> ProbeFuture {
                Box::pin(async {
                    let _permit = probe_permit().await;
                    probe_claude_runtime().await
                })
            }),
        )
    }

    #[cfg(test)]
    fn with_probe(
        app_local_data: PathBuf,
        secrets: Arc<dyn SecretStore>,
        claude_probe: Probe,
    ) -> Self {
        let work_root = app_local_data.join(WORK_ROOT_DIR);
        Self::assemble(app_local_data, secrets, work_root, claude_probe)
    }

    fn assemble(
        app_local_data: PathBuf,
        secrets: Arc<dyn SecretStore>,
        work_root: PathBuf,
        claude_probe: Probe,
    ) -> Self {
        Self {
            settings: PlanSettingsStore::new(&app_local_data),
            app_local_data,
            models: Arc::new(PlanModelSource::new()),
            secrets,
            work_root,
            claude_probe,
            probe_cache: Arc::new(Mutex::new(None)),
            probe_lock: Arc::new(AsyncMutex::new(())),
            sessions: Mutex::new(HashMap::new()),
            sign_ins: Mutex::new(HashMap::new()),
            refresh_lock: AsyncMutex::new(()),
        }
    }

    pub(crate) fn model_source(&self) -> Arc<PlanModelSource> {
        Arc::clone(&self.models)
    }

    /// With Claude off this returns the disabled view and starts no process.
    /// `refresh` skips the cached probe.
    pub(crate) async fn claude_status(&self, refresh: bool) -> ClaudeRuntimeStatusView {
        if !self.claude_enabled() {
            self.models.clear_claude();
            return claude_disabled_view(self.claude_effort());
        }
        let (probed_at, probe) = self.cached_claude_probe(refresh).await;
        self.apply_claude_probe(probed_at, &probe)
    }

    /// Writes the Claude rows for `probe`, or clears them when Claude is off. `Ready`
    /// offers them from `probed_at`. A failed check keeps the rows as they are, since
    /// a transient failure is not a missing install. Any other state clears them. The
    /// check and the write hold `probe_cache`, which `claude_set_enabled` also holds
    /// while it saves the setting. A disable cannot land between them, so a probe
    /// that finishes after one is not written back.
    fn apply_claude_probe(
        &self,
        probed_at: Instant,
        probe: &ProbeResult,
    ) -> ClaudeRuntimeStatusView {
        let _gate = lock(&self.probe_cache);
        if !self.claude_enabled() {
            self.models.clear_claude();
            return claude_disabled_view(self.claude_effort());
        }
        match probe.state {
            ProbeState::Ready => self.models.set_claude_at(claude_rows(), probed_at),
            ProbeState::CheckFailed => {}
            ProbeState::NotInstalled | ProbeState::Outdated | ProbeState::SignedOut => {
                self.models.clear_claude();
            }
        }
        claude_view(probe, self.claude_effort())
    }

    pub(crate) async fn claude_set_enabled(
        &self,
        enabled: bool,
    ) -> Result<ClaudeRuntimeStatusView, PlanProviderError> {
        {
            // Waits for a refresh in flight, so the refresh cannot write rows after
            // this change. The setting and the cached probe change under `probe_cache`.
            let _serial = self.refresh_lock.lock().await;
            let mut cache = lock(&self.probe_cache);
            self.settings
                .set_claude_enabled(enabled)
                .map_err(|_| PlanProviderError::Settings)?;
            *cache = None;
        }
        Ok(self.claude_status(true).await)
    }

    /// Saves the level a new chat starts with. It works while Claude is off, and then the
    /// returned view is the disabled one carrying the level. The cached probe stays, since
    /// the level changes no probe answer.
    pub(crate) async fn claude_set_effort(
        &self,
        effort: ClaudeEffort,
    ) -> Result<ClaudeRuntimeStatusView, PlanProviderError> {
        {
            // Holds the locks `claude_set_enabled` holds, so two settings writes cannot
            // interleave.
            let _serial = self.refresh_lock.lock().await;
            let _cache = lock(&self.probe_cache);
            self.settings
                .set_claude_effort(effort)
                .map_err(|_| PlanProviderError::Settings)?;
        }
        Ok(self.claude_status(false).await)
    }

    pub(crate) async fn chatgpt_status(
        &self,
        account: Option<&ValidatedAccountId>,
    ) -> ChatgptPlanStatusView {
        self.status_view(account).await
    }

    /// A sign-in already running for the account is reported, not started again.
    pub(crate) async fn chatgpt_sign_in(
        &self,
        account: &ValidatedAccountId,
        opener: &dyn BrowserOpener,
    ) -> ChatgptPlanStatusView {
        let Some(session) = self.session_for(account) else {
            return self.status_view(Some(account)).await;
        };
        let cancel = CancellationToken::new();
        let Some(claim) = SignInClaim::take(&self.sign_ins, account.as_str(), cancel.clone())
        else {
            return self.status_view(Some(account)).await;
        };
        let outcome = session.sign_in(opener, &cancel, now_unix()).await;
        // The slot is freed before the status is read, so the reply does not
        // still say SigningIn.
        drop(claim);
        match outcome {
            Ok(_) => {
                self.refresh_models(account).await;
                self.status_view(Some(account)).await
            }
            Err(error) => failed_sign_in(self.status_view(Some(account)).await, error),
        }
    }

    pub(crate) fn chatgpt_sign_in_cancel(&self, account: &ValidatedAccountId) {
        if let Some(cancel) = lock(&self.sign_ins).get(account.as_str()) {
            cancel.cancel();
        }
    }

    pub(crate) async fn chatgpt_sign_out(
        &self,
        account: &ValidatedAccountId,
    ) -> ChatgptPlanStatusView {
        self.chatgpt_sign_in_cancel(account);
        // Read before the lock, so a keychain prompt here does not hold up a refresh
        // or a Claude change. With nothing stored there is nothing to revoke. Calling
        // sign_out would report Unconfirmed and show a text about a revocation that
        // was never needed. A read that did not finish is not taken for nothing
        // stored, so the sign-out runs.
        let nothing_stored = match self.session_for(account) {
            Some(session) => {
                blocking_read(move || session.status().state).await == Some(SessionState::SignedOut)
            }
            None => true,
        };
        // Waits for a refresh in flight, so the refresh cannot write rows after this
        // sign-out clears them.
        let _serial = self.refresh_lock.lock().await;
        let session = match (nothing_stored, self.session_for(account)) {
            (false, Some(session)) => session,
            _ => {
                self.models.clear_chatgpt(account.as_str());
                return self.status_view(Some(account)).await;
            }
        };
        let outcome = session.sign_out().await;
        self.models.clear_chatgpt(account.as_str());
        let status = self.status_view(Some(account)).await;
        match outcome {
            RevokeOutcome::Confirmed => status,
            RevokeOutcome::Unconfirmed => ChatgptPlanStatusView {
                error: Some(SIGN_OUT_UNCONFIRMED_TEXT.to_owned()),
                ..status
            },
        }
    }

    /// Synchronous, because its command is, so these store reads run on the calling
    /// thread.
    pub(crate) fn chatgpt_welcome_ack(
        &self,
        account: &ValidatedAccountId,
    ) -> ChatgptPlanStatusView {
        let Some(session) = self.session_for(account) else {
            return self.view_from(account, None);
        };
        let acknowledged = session.acknowledge_welcome();
        let status = session.status();
        match acknowledged {
            Ok(()) => self.view_from(account, Some(status)),
            Err(error) => ChatgptPlanStatusView {
                error: session_error_text(error).map(str::to_owned),
                ..self.view_from(account, Some(status))
            },
        }
    }

    /// Replaces the account's ChatGPT rows with what the account can use now.
    /// With Claude off and no ChatGPT sign-in, this reads two small local files
    /// and sends no request.
    pub(crate) async fn refresh_models(&self, account: &ValidatedAccountId) {
        let _serial = self.refresh_lock.lock().await;
        self.claude_status(false).await;
        self.refresh_chatgpt_models(account).await;
    }

    /// Renews the Claude rows before a read that selects a model from the catalogue.
    /// A probe answer still fresh for its TTL is applied with no process. With rows
    /// offered, a stale answer starts a probe in the background, and the read goes on
    /// with the rows it has. With rows offered, the read never waits for a refresh: if
    /// `refresh_lock` is held, a refresh is in flight and applies Claude's state when it
    /// finishes, so the read returns at once. With no rows offered, the read waits for
    /// the refresh and the probe, up to `RENEW_LIMIT`. A probe that runs past that limit
    /// is not cancelled: it still caches its answer, and the next read applies it. With
    /// Claude off this returns at once, since the rows are cleared when it is turned
    /// off.
    pub(crate) async fn renew_claude_rows(&self) {
        if !self.claude_enabled() {
            return;
        }
        let rows_offered = self.models.has_claude_rows();
        let fresh = self.fresh_claude_probe(false, Instant::now()).is_some();
        if rows_offered && !fresh {
            self.start_claude_probe_in_background();
            return;
        }
        let renewed = if rows_offered {
            // A refresh holds `refresh_lock` while it waits on ChatGPT's network, so
            // the read does not wait for it.
            let Ok(_serial) = self.refresh_lock.try_lock() else {
                return;
            };
            tokio::time::timeout(RENEW_LIMIT, self.claude_status(false))
                .await
                .map(|_| ())
        } else {
            tokio::time::timeout(RENEW_LIMIT, async {
                let _serial = self.refresh_lock.lock().await;
                self.claude_status(false).await;
            })
            .await
        };
        if renewed.is_err() {
            tracing::warn!("the Claude model renewal ran past its limit");
        }
    }

    /// Cancels every sign-in in flight, drops the cached sessions and clears
    /// the ChatGPT rows. Claude is a device setting, so its rows stay.
    pub(crate) fn forget_account(&self) {
        for cancel in lock(&self.sign_ins).values() {
            cancel.cancel();
        }
        lock(&self.sessions).clear();
        self.models.clear_all_chatgpt();
    }

    pub(crate) fn credential(
        &self,
        provider: ProviderId,
        account: &ValidatedAccountId,
    ) -> Result<ResolvedCredential, String> {
        match provider {
            ProviderId::ClaudeCode => {
                if !self.claude_enabled() {
                    return Err(CLAUDE_OFF_TEXT.to_owned());
                }
                Ok(plan_credential("claude-account"))
            }
            ProviderId::ChatgptPlan => {
                let state = self
                    .session_for(account)
                    .map(|session| session.status().state);
                match state {
                    Some(SessionState::Ready) => Ok(plan_credential("chatgpt-plan")),
                    Some(SessionState::CheckFailed) => Err(CHATGPT_CHECK_FAILED_TEXT.to_owned()),
                    _ => Err(CHATGPT_SIGN_IN_TEXT.to_owned()),
                }
            }
            ProviderId::OpenAi
            | ProviderId::Anthropic
            | ProviderId::Google
            | ProviderId::OpenRouter
            | ProviderId::CustomOpenAi => Err(NOT_A_PLAN_TEXT.to_owned()),
        }
    }

    /// Refused while Claude is off, so a stream cannot start a process that
    /// `credential` would have refused. The first adapter built for a conversation
    /// fixes its Claude level for good; later ones reuse it.
    pub(crate) fn adapter(
        &self,
        provider: ProviderId,
        account: &ValidatedAccountId,
        conversation: Option<&str>,
    ) -> Result<Arc<dyn ProviderAdapter>, ProviderError> {
        match provider {
            ProviderId::ClaudeCode => {
                if !self.claude_enabled() {
                    return Err(ProviderError::RuntimeUnavailable);
                }
                let current = self.claude_effort();
                let effort = match conversation {
                    Some(id) => {
                        ChatEffortStore::new(&self.app_local_data, account).lock_or_get(id, current)
                    }
                    None => current,
                };
                let runtime = ClaudeRuntimeAdapter::new(self.work_root.clone()).with_effort(effort);
                Ok(with_retry(Arc::new(LimitedAdapter::new(Arc::new(runtime)))))
            }
            ProviderId::ChatgptPlan => {
                let session = self
                    .session_for(account)
                    .ok_or(ProviderError::PlanSignInRequired)?;
                let adapter = SessionBackedChatgptAdapter::new(
                    session,
                    Arc::clone(&self.models),
                    account.as_str().to_owned(),
                )
                .with_conversation(conversation);
                Ok(with_retry(Arc::new(adapter)))
            }
            ProviderId::OpenAi
            | ProviderId::Anthropic
            | ProviderId::Google
            | ProviderId::OpenRouter
            | ProviderId::CustomOpenAi => Err(ProviderError::InvalidResponse),
        }
    }

    fn claude_enabled(&self) -> bool {
        self.settings.load().claude_enabled
    }

    pub(crate) fn claude_effort(&self) -> ClaudeEffort {
        self.settings.load().claude_effort
    }

    /// The probe answer to use, with the moment it finished. A fresh answer is reused.
    /// Otherwise the probe runs in its own task, so a caller that stops waiting (a
    /// timeout, or a drop) does not lose the answer: the task still caches it and
    /// releases the lock.
    async fn cached_claude_probe(&self, refresh: bool) -> (Instant, ProbeResult) {
        let requested = Instant::now();
        let guard = Arc::clone(&self.probe_lock).lock_owned().await;
        // A refresh takes an answer only from a probe that finished after this call
        // began, which is one that was in flight when the call arrived.
        if let Some(cached) = self.fresh_claude_probe(refresh, requested) {
            return cached;
        }
        match self.spawn_claude_probe(guard).await {
            Ok(answer) => answer,
            Err(error) => {
                tracing::warn!(
                    panicked = error.is_panic(),
                    "the Claude probe did not finish; reporting a failed check"
                );
                (Instant::now(), check_failed_probe())
            }
        }
    }

    /// The cached answer, when it is fresh for its own TTL. With `refresh`, it must
    /// also have finished at or after `requested`.
    fn fresh_claude_probe(
        &self,
        refresh: bool,
        requested: Instant,
    ) -> Option<(Instant, ProbeResult)> {
        lock(&self.probe_cache)
            .as_ref()
            .filter(|(at, probe)| at.elapsed() < probe_ttl(probe) && (!refresh || *at >= requested))
            .cloned()
    }

    /// Runs the probe in its own task. The task holds `guard` until the answer is
    /// cached, so the lock is released only after the cache is written. Dropping the
    /// handle detaches the task, which runs on to the end.
    fn spawn_claude_probe(&self, guard: OwnedMutexGuard<()>) -> JoinHandle<(Instant, ProbeResult)> {
        let probe = Arc::clone(&self.claude_probe);
        let cache = Arc::clone(&self.probe_cache);
        tokio::spawn(async move {
            let _serial = guard;
            let answer = probe().await;
            let probed_at = Instant::now();
            *lock(&cache) = Some((probed_at, answer.clone()));
            (probed_at, answer)
        })
    }

    /// Starts a probe without waiting for it, when the cached answer is stale and no
    /// probe is running. The lock is tried, never awaited, so this does not wait for a
    /// probe in flight. The answer is cached when it finishes, and the next read
    /// applies it.
    fn start_claude_probe_in_background(&self) {
        let Ok(guard) = Arc::clone(&self.probe_lock).try_lock_owned() else {
            return;
        };
        if self.fresh_claude_probe(false, Instant::now()).is_some() {
            return;
        }
        let _detached = self.spawn_claude_probe(guard);
    }

    async fn refresh_chatgpt_models(&self, account: &ValidatedAccountId) {
        let account_id = account.as_str();
        let Some(session) = self.session_for(account) else {
            self.models.clear_chatgpt(account_id);
            return;
        };
        // A read that did not finish is not Ready, so the rows are cleared.
        let status_session = Arc::clone(&session);
        if blocking_read(move || status_session.status().state).await != Some(SessionState::Ready) {
            self.models.clear_chatgpt(account_id);
            return;
        }
        match session.offered_models(now_unix()).await {
            Ok(offered) => self.models.set_chatgpt(account_id, chatgpt_rows(&offered)),
            Err(ProviderError::PlanSignInRequired | ProviderError::PlanNotEligible) => {
                self.models.clear_chatgpt(account_id);
            }
            // A transient failure keeps the list already offered, so the picker
            // does not lose models on a network blip.
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not refresh the ChatGPT plan models; keeping the list already offered"
                );
            }
        }
    }

    /// The account's session, built on first use. `None` only when the built-in
    /// OAuth configuration cannot be made.
    fn session_for(&self, account: &ValidatedAccountId) -> Option<Arc<ChatgptPlanSession>> {
        let mut sessions = lock(&self.sessions);
        if let Some(session) = sessions.get(account.as_str()) {
            return Some(Arc::clone(session));
        }
        let store = PlanStore::new(&self.app_local_data, account, Arc::clone(&self.secrets));
        let session = Arc::new(ChatgptPlanSession::production(store)?);
        sessions.insert(account.as_str().to_owned(), Arc::clone(&session));
        Some(session)
    }

    /// Reads local state only. It never sends a request. The store read runs on a
    /// blocking thread, so a keychain prompt cannot hold an async worker.
    async fn status_view(&self, account: Option<&ValidatedAccountId>) -> ChatgptPlanStatusView {
        let Some(account) = account else {
            return chatgpt_view(ChatgptPlanState::SignedOut, false, None, 0, None);
        };
        let status = self.read_status(account).await;
        self.view_from(account, status)
    }

    /// The view for `account` from a status the caller read. `None` is a status that
    /// could not be read at all, which reads as a failed check.
    fn view_from(
        &self,
        account: &ValidatedAccountId,
        status: Option<SessionStatus>,
    ) -> ChatgptPlanStatusView {
        let model_count =
            u32::try_from(self.models.chatgpt_count(account.as_str())).unwrap_or(u32::MAX);
        if lock(&self.sign_ins).contains_key(account.as_str()) {
            return chatgpt_view(ChatgptPlanState::SigningIn, false, None, model_count, None);
        }
        match status {
            Some(status) => chatgpt_view(
                chatgpt_state(status.state),
                status.welcome_pending,
                status.account_label,
                model_count,
                None,
            ),
            None => chatgpt_view(
                ChatgptPlanState::CheckFailed,
                false,
                None,
                model_count,
                Some(CHATGPT_UNAVAILABLE_TEXT),
            ),
        }
    }

    /// The account's session status, read on a blocking thread. `None` when there is
    /// no session or the read did not finish.
    async fn read_status(&self, account: &ValidatedAccountId) -> Option<SessionStatus> {
        let session = self.session_for(account)?;
        blocking_read(move || session.status()).await
    }
}

/// Runs a store or keychain read on a blocking thread, so a macOS Keychain prompt
/// cannot hold an async worker. `None` when the read panicked or was cancelled. A
/// caller treats that as a failed read, never as a value.
async fn blocking_read<T: Send + 'static>(read: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    match tokio::task::spawn_blocking(read).await {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(
                panicked = error.is_panic(),
                "a ChatGPT plan store read did not finish"
            );
            None
        }
    }
}

/// Holds an account's sign-in slot. Dropping it frees the slot, including when
/// the sign-in is dropped before it finishes.
struct SignInClaim<'a> {
    sign_ins: &'a Mutex<HashMap<String, CancellationToken>>,
    account: String,
}

impl<'a> SignInClaim<'a> {
    /// `None` when the account already has a sign-in in flight.
    fn take(
        sign_ins: &'a Mutex<HashMap<String, CancellationToken>>,
        account: &str,
        cancel: CancellationToken,
    ) -> Option<Self> {
        let mut in_flight = lock(sign_ins);
        if in_flight.contains_key(account) {
            return None;
        }
        in_flight.insert(account.to_owned(), cancel);
        Some(Self {
            sign_ins,
            account: account.to_owned(),
        })
    }
}

impl Drop for SignInClaim<'_> {
    fn drop(&mut self) {
        lock(self.sign_ins).remove(&self.account);
    }
}

/// The guarded state is plain data, and no guard is held across an await, so a
/// poisoned lock still holds a usable value.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// A turn directory is removed by its own run, so one still here came from a
/// crash. A symbolic link is left alone, so removing it cannot reach its target.
fn sweep_stale_turns(work_root: &Path) {
    let Ok(entries) = fs::read_dir(work_root) else {
        return;
    };
    // Best effort: an entry that cannot be read is not a reason to stop the
    // sweep of the others.
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(TURN_DIR_PREFIX)
        {
            continue;
        }
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        if let Err(error) = fs::remove_dir_all(entry.path()) {
            tracing::warn!(
                kind = ?error.kind(),
                "could not remove a stale Claude turn directory"
            );
        }
    }
}

fn plan_credential(credential_id: &str) -> ResolvedCredential {
    ResolvedCredential {
        credential_id: credential_id.to_owned(),
        secret: Zeroizing::new(String::new()),
        base_url: None,
    }
}

/// How long `probe` is reused. A failed check is checked again sooner.
fn probe_ttl(probe: &ProbeResult) -> Duration {
    if probe.state == ProbeState::CheckFailed {
        CHECK_FAILED_PROBE_TTL
    } else {
        PROBE_TTL
    }
}

/// The answer for a probe that did not finish, such as a panicked task.
fn check_failed_probe() -> ProbeResult {
    ProbeResult {
        state: ProbeState::CheckFailed,
        installed_version: None,
        plan: None,
        auth_method: None,
    }
}

fn claude_state(state: ProbeState) -> ClaudeRuntimeState {
    match state {
        ProbeState::NotInstalled => ClaudeRuntimeState::NotInstalled,
        ProbeState::Outdated => ClaudeRuntimeState::Outdated,
        ProbeState::SignedOut => ClaudeRuntimeState::SignedOut,
        ProbeState::Ready => ClaudeRuntimeState::Ready,
        ProbeState::CheckFailed => ClaudeRuntimeState::CheckFailed,
    }
}

fn claude_view(probe: &ProbeResult, effort: ClaudeEffort) -> ClaudeRuntimeStatusView {
    ClaudeRuntimeStatusView {
        version: CONTRACT_VERSION,
        enabled: true,
        state: claude_state(probe.state),
        installed_version: probe.installed_version.as_ref().map(ToString::to_string),
        required_version: MIN_CLAUDE_VERSION.to_string(),
        plan: probe.plan.clone(),
        sign_in_method: probe.auth_method.clone(),
        sign_in_command: CLAUDE_SIGN_IN_COMMAND.to_owned(),
        effort,
    }
}

/// Nothing is probed or reported as installed while Claude is off. The effort is
/// still reported, since a level can be set while Claude is off.
fn claude_disabled_view(effort: ClaudeEffort) -> ClaudeRuntimeStatusView {
    ClaudeRuntimeStatusView {
        version: CONTRACT_VERSION,
        enabled: false,
        state: ClaudeRuntimeState::Disabled,
        installed_version: None,
        required_version: MIN_CLAUDE_VERSION.to_string(),
        plan: None,
        sign_in_method: None,
        sign_in_command: CLAUDE_SIGN_IN_COMMAND.to_owned(),
        effort,
    }
}

fn chatgpt_state(state: SessionState) -> ChatgptPlanState {
    match state {
        SessionState::SignedOut => ChatgptPlanState::SignedOut,
        SessionState::Ready => ChatgptPlanState::Ready,
        SessionState::PlanUsageDisabled => ChatgptPlanState::PlanUsageDisabled,
        SessionState::NeedsSignIn => ChatgptPlanState::NeedsSignIn,
        SessionState::CheckFailed => ChatgptPlanState::CheckFailed,
    }
}

fn chatgpt_view(
    state: ChatgptPlanState,
    welcome_pending: bool,
    account_label: Option<String>,
    model_count: u32,
    error: Option<&str>,
) -> ChatgptPlanStatusView {
    ChatgptPlanStatusView {
        version: CONTRACT_VERSION,
        state,
        welcome_pending,
        account_label,
        model_count,
        usage_url: CHATGPT_USAGE_URL.to_owned(),
        error: error.map(str::to_owned),
    }
}

/// A cancelled sign-in is the user's own choice, so it has no error text. A
/// session that is already Ready keeps its state, and only the text is added.
fn failed_sign_in(status: ChatgptPlanStatusView, error: SessionError) -> ChatgptPlanStatusView {
    let Some(text) = session_error_text(error) else {
        return status;
    };
    let state = if status.state == ChatgptPlanState::Ready {
        ChatgptPlanState::Ready
    } else {
        ChatgptPlanState::CheckFailed
    };
    ChatgptPlanStatusView {
        state,
        error: Some(text.to_owned()),
        ..status
    }
}

fn session_error_text(error: SessionError) -> Option<&'static str> {
    match error {
        SessionError::Cancelled => None,
        SessionError::Declined => Some("Sign-in was declined."),
        SessionError::TimedOut => Some("Sign-in timed out. Try again."),
        SessionError::Rejected => Some("The sign-in response could not be verified. Try again."),
        SessionError::RegistrationRejected => {
            Some("OpenAI did not accept this app's sign-in registration. Try again.")
        }
        SessionError::Network => {
            Some("Could not reach OpenAI. Check your connection and try again.")
        }
        SessionError::Browser => Some("Could not open your browser."),
        SessionError::Store => Some("Could not save the sign-in on this device."),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use reqwest::Url;
    use serde_json::json;
    use tokio::sync::Notify;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::clients::da::model_catalog::LocalModelRows;
    use crate::persistence::InMemorySecretStore;
    use crate::plan_providers::chat_effort::CHAT_EFFORT_FILE;
    use crate::plan_providers::models::PlanModelRow;
    use crate::providers::chatgpt_plan::oauth::{OAuthConfig, PLAN_USAGE_SCOPE};
    use crate::providers::chatgpt_plan::session::BrowserOpenError;
    use crate::providers::chatgpt_plan::store::{Meta, Profile, SessionRecord};
    use crate::providers::claude_runtime::adapter::CLAUDE_MODELS;
    use crate::security::account_cleanup::account_directory_name;

    const ACCOUNT: &str = "person:tenant";
    const CLIENT: &str = "oaiapp_service_client";
    const SUBJECT: &str = "user-service";

    fn account() -> ValidatedAccountId {
        ValidatedAccountId::parse(ACCOUNT).expect("account")
    }

    fn probe_answering(state: ProbeState) -> ProbeResult {
        ProbeResult {
            state,
            installed_version: None,
            plan: None,
            auth_method: None,
        }
    }

    fn ready_probe() -> ProbeResult {
        ProbeResult {
            state: ProbeState::Ready,
            installed_version: Some(MIN_CLAUDE_VERSION),
            plan: Some("max".to_owned()),
            auth_method: Some("subscription".to_owned()),
        }
    }

    /// Answers each probe with the current answer, and counts the probes.
    struct ScriptedProbe {
        answer: Arc<Mutex<ProbeResult>>,
        calls: Arc<AtomicUsize>,
    }

    impl ScriptedProbe {
        fn new(answer: ProbeResult) -> Self {
            Self {
                answer: Arc::new(Mutex::new(answer)),
                calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn answer(&self, answer: ProbeResult) {
            *lock(&self.answer) = answer;
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn probe(&self) -> Probe {
            let answer = Arc::clone(&self.answer);
            let calls = Arc::clone(&self.calls);
            Arc::new(move || -> ProbeFuture {
                calls.fetch_add(1, Ordering::SeqCst);
                let result = lock(&answer).clone();
                Box::pin(std::future::ready(result))
            })
        }
    }

    fn providers_for(
        root: &Path,
        secrets: &Arc<InMemorySecretStore>,
        probe: &ScriptedProbe,
    ) -> PlanProviders {
        PlanProviders::with_probe(root.to_path_buf(), secrets.clone(), probe.probe())
    }

    fn enable_claude(root: &Path) {
        PlanSettingsStore::new(root)
            .set_claude_enabled(true)
            .expect("settings");
    }

    /// The file `ChatEffortStore` keeps the conversation levels of `account` in.
    fn chat_effort_file(root: &Path, account: &ValidatedAccountId) -> PathBuf {
        root.join("accounts")
            .join(account_directory_name(account.as_str()))
            .join(CHAT_EFFORT_FILE)
    }

    /// A ChatGPT sign-in on disk, as an earlier run would have left it.
    fn seed_sign_in(
        root: &Path,
        secrets: &Arc<InMemorySecretStore>,
        scopes: &[&str],
        welcome_acknowledged: bool,
    ) {
        let store = PlanStore::new(root, &account(), secrets.clone());
        store
            .save_session(&SessionRecord {
                client_id: CLIENT.to_owned(),
                subject: SUBJECT.to_owned(),
                email: None,
                id_token: None,
                access_token: "access-service".to_owned(),
                refresh_token: Some("refresh-service".to_owned()),
                expires_at: now_unix() + 3_600,
                earliest_refresh_at: None,
                scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
                saved_at: now_unix(),
            })
            .expect("session");
        store
            .save_meta(&Meta {
                host_id: None,
                active_client_id: Some(CLIENT.to_owned()),
                welcome_acknowledged,
                profiles: vec![Profile {
                    client_id: CLIENT.to_owned(),
                    subject: SUBJECT.to_owned(),
                    email: None,
                }],
            })
            .expect("meta");
    }

    fn chatgpt_row(slug: &str) -> PlanModelRow {
        PlanModelRow {
            provider: "chatgpt_plan",
            slug: slug.to_owned(),
            display_name: slug.to_owned(),
            context_length: 1_050_000,
            max_output_tokens: 128_000,
            supports_vision: true,
        }
    }

    /// Refuses to open a browser. A test that reaches it has started a sign-in.
    struct NoBrowser;

    impl BrowserOpener for NoBrowser {
        fn open(&self, _url: &Url) -> Result<(), BrowserOpenError> {
            Err(BrowserOpenError)
        }
    }

    #[test]
    fn each_probe_state_maps_to_its_claude_state() {
        assert_eq!(
            claude_state(ProbeState::NotInstalled),
            ClaudeRuntimeState::NotInstalled
        );
        assert_eq!(
            claude_state(ProbeState::Outdated),
            ClaudeRuntimeState::Outdated
        );
        assert_eq!(
            claude_state(ProbeState::SignedOut),
            ClaudeRuntimeState::SignedOut
        );
        assert_eq!(claude_state(ProbeState::Ready), ClaudeRuntimeState::Ready);
        assert_eq!(
            claude_state(ProbeState::CheckFailed),
            ClaudeRuntimeState::CheckFailed
        );
    }

    #[test]
    fn each_session_state_maps_to_its_chatgpt_state() {
        assert_eq!(
            chatgpt_state(SessionState::SignedOut),
            ChatgptPlanState::SignedOut
        );
        assert_eq!(chatgpt_state(SessionState::Ready), ChatgptPlanState::Ready);
        assert_eq!(
            chatgpt_state(SessionState::PlanUsageDisabled),
            ChatgptPlanState::PlanUsageDisabled
        );
        assert_eq!(
            chatgpt_state(SessionState::NeedsSignIn),
            ChatgptPlanState::NeedsSignIn
        );
        assert_eq!(
            chatgpt_state(SessionState::CheckFailed),
            ChatgptPlanState::CheckFailed
        );
    }

    #[test]
    fn the_disabled_claude_view_names_no_install_and_the_sign_in_command() {
        let view = claude_disabled_view(ClaudeEffort::High);

        assert_eq!(view.version, CONTRACT_VERSION);
        assert!(!view.enabled);
        assert_eq!(view.state, ClaudeRuntimeState::Disabled);
        assert_eq!(view.installed_version, None);
        assert_eq!(view.required_version, MIN_CLAUDE_VERSION.to_string());
        assert_eq!(view.sign_in_command, "claude auth login");
    }

    #[test]
    fn a_ready_probe_maps_its_version_plan_and_sign_in_method() {
        let view = claude_view(&ready_probe(), ClaudeEffort::High);

        assert!(view.enabled);
        assert_eq!(view.state, ClaudeRuntimeState::Ready);
        assert_eq!(view.installed_version, Some(MIN_CLAUDE_VERSION.to_string()));
        assert_eq!(view.required_version, MIN_CLAUDE_VERSION.to_string());
        assert_eq!(view.plan.as_deref(), Some("max"));
        assert_eq!(view.sign_in_method.as_deref(), Some("subscription"));
    }

    #[test]
    fn each_sign_in_failure_has_its_fixed_text_and_a_cancel_has_none() {
        assert_eq!(session_error_text(SessionError::Cancelled), None);
        assert_eq!(
            session_error_text(SessionError::Declined),
            Some("Sign-in was declined.")
        );
        assert_eq!(
            session_error_text(SessionError::TimedOut),
            Some("Sign-in timed out. Try again.")
        );
        assert_eq!(
            session_error_text(SessionError::Rejected),
            Some("The sign-in response could not be verified. Try again.")
        );
        assert_eq!(
            session_error_text(SessionError::RegistrationRejected),
            Some("OpenAI did not accept this app's sign-in registration. Try again.")
        );
        assert_eq!(
            session_error_text(SessionError::Network),
            Some("Could not reach OpenAI. Check your connection and try again.")
        );
        assert_eq!(
            session_error_text(SessionError::Browser),
            Some("Could not open your browser.")
        );
        assert_eq!(
            session_error_text(SessionError::Store),
            Some("Could not save the sign-in on this device.")
        );
    }

    #[test]
    fn a_failed_sign_in_keeps_ready_and_otherwise_reports_check_failed() {
        let signed_out = chatgpt_view(ChatgptPlanState::SignedOut, false, None, 0, None);
        let failed = failed_sign_in(signed_out.clone(), SessionError::Network);
        assert_eq!(failed.state, ChatgptPlanState::CheckFailed);
        assert_eq!(
            failed.error.as_deref(),
            Some("Could not reach OpenAI. Check your connection and try again.")
        );

        let ready = chatgpt_view(ChatgptPlanState::Ready, false, None, 3, None);
        assert_eq!(
            failed_sign_in(ready, SessionError::Store).state,
            ChatgptPlanState::Ready
        );

        assert_eq!(
            failed_sign_in(signed_out.clone(), SessionError::Cancelled),
            signed_out
        );
    }

    #[tokio::test]
    async fn with_claude_off_no_probe_runs_and_the_rows_are_cleared() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        providers.model_source().set_claude(claude_rows());

        let status = providers.claude_status(true).await;
        providers.refresh_models(&account()).await;

        assert_eq!(status.state, ClaudeRuntimeState::Disabled);
        assert!(!status.enabled);
        assert_eq!(probe.calls(), 0);
        assert!(providers.model_source().rows(ACCOUNT).is_empty());
    }

    #[tokio::test]
    async fn a_ready_probe_offers_the_claude_rows_and_a_missing_install_clears_them() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        providers.claude_status(false).await;
        assert_eq!(
            providers.model_source().rows(ACCOUNT).len(),
            CLAUDE_MODELS.len()
        );

        probe.answer(probe_answering(ProbeState::NotInstalled));
        let status = providers.claude_status(true).await;

        assert_eq!(status.state, ClaudeRuntimeState::NotInstalled);
        assert!(providers.model_source().rows(ACCOUNT).is_empty());
    }

    #[tokio::test]
    async fn the_probe_is_cached_for_its_ttl_and_refresh_bypasses_it() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        providers.claude_status(false).await;
        providers.claude_status(false).await;
        assert_eq!(probe.calls(), 1);

        providers.claude_status(true).await;
        assert_eq!(probe.calls(), 2);
    }

    /// A send after the TTL renews the rows: the expired probe runs again in the
    /// background, and the next read applies its answer. The rows stay offered until
    /// then.
    #[tokio::test]
    async fn renewing_for_a_send_reprobes_an_expired_probe_and_offers_the_rows_again() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        providers.claude_status(false).await;
        assert_eq!(probe.calls(), 1);

        // The cached probe and the rows it wrote are both older than the TTL.
        let aged = Instant::now() - (PROBE_TTL + Duration::from_secs(1));
        *lock(&providers.probe_cache) = Some((aged, ready_probe()));
        providers.model_source().set_claude_at(claude_rows(), aged);
        assert_eq!(
            providers.model_source().rows(ACCOUNT).len(),
            CLAUDE_MODELS.len()
        );

        providers.renew_claude_rows().await;
        // This read waits for the background probe, then applies its answer.
        providers.claude_status(false).await;

        assert_eq!(probe.calls(), 2);
        assert_eq!(
            providers.model_source().rows(ACCOUNT).len(),
            CLAUDE_MODELS.len()
        );
    }

    /// A send within the TTL of the last probe reuses that probe, so it starts no
    /// process.
    #[tokio::test]
    async fn renewing_within_the_ttl_reuses_the_probe_and_starts_none() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        providers.claude_status(false).await;

        providers.renew_claude_rows().await;
        providers.renew_claude_rows().await;

        assert_eq!(probe.calls(), 1);
        assert_eq!(
            providers.model_source().rows(ACCOUNT).len(),
            CLAUDE_MODELS.len()
        );
    }

    /// Turning Claude off clears its rows, and a renewal after that starts no probe.
    #[tokio::test]
    async fn renewing_with_claude_off_starts_no_probe_and_leaves_no_claude_rows() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        providers.claude_status(false).await;
        providers.claude_set_enabled(false).await.expect("saved");
        assert_eq!(probe.calls(), 1);

        providers.renew_claude_rows().await;

        assert_eq!(probe.calls(), 1);
        assert!(providers.model_source().rows(ACCOUNT).is_empty());
    }

    /// Moves the cached answer's finish time back by `age`, as if that probe had
    /// finished `age` ago.
    fn age_cached_probe(providers: &PlanProviders, age: Duration) {
        if let Some((probed_at, _)) = lock(&providers.probe_cache).as_mut() {
            *probed_at = Instant::now() - age;
        }
    }

    /// A read with rows offered does not wait for a stale probe. The probe runs in
    /// the background, the rows stay offered meanwhile, and a later read applies the
    /// answer.
    #[tokio::test]
    async fn a_read_with_rows_offered_does_not_wait_for_a_stale_probe() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let providers = PlanProviders::with_probe(
            root.path().to_path_buf(),
            secrets,
            gated_probe(
                Arc::clone(&started),
                Arc::clone(&release),
                probe_answering(ProbeState::SignedOut),
                Arc::clone(&calls),
            ),
        );
        let aged = Instant::now() - (PROBE_TTL + Duration::from_secs(1));
        *lock(&providers.probe_cache) = Some((aged, ready_probe()));
        providers.model_source().set_claude_at(claude_rows(), aged);

        tokio::time::timeout(Duration::from_secs(1), async {
            providers.renew_claude_rows().await;
            providers.renew_claude_rows().await;
        })
        .await
        .expect("the reads did not wait for the stale probe");
        started.notified().await;

        assert_eq!(calls.load(Ordering::SeqCst), 1, "one background probe");
        assert_eq!(
            providers.model_source().rows(ACCOUNT).len(),
            CLAUDE_MODELS.len()
        );

        release.notify_one();
        let status = providers.claude_status(false).await;

        assert_eq!(status.state, ClaudeRuntimeState::SignedOut);
        assert!(providers.model_source().rows(ACCOUNT).is_empty());
    }

    /// A failed check keeps the rows that were offered, and is checked again only
    /// after `CHECK_FAILED_PROBE_TTL`, not sooner.
    #[tokio::test]
    async fn a_failed_check_keeps_the_rows_and_is_checked_again_after_its_short_ttl() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        providers.claude_status(false).await;

        probe.answer(probe_answering(ProbeState::CheckFailed));
        let failed = providers.claude_status(true).await;

        assert_eq!(failed.state, ClaudeRuntimeState::CheckFailed);
        assert_eq!(probe.calls(), 2);
        assert_eq!(
            providers.model_source().rows(ACCOUNT).len(),
            CLAUDE_MODELS.len()
        );

        age_cached_probe(&providers, CHECK_FAILED_PROBE_TTL - Duration::from_secs(1));
        providers.claude_status(false).await;
        assert_eq!(probe.calls(), 2, "inside its TTL a failed check is reused");

        age_cached_probe(&providers, CHECK_FAILED_PROBE_TTL + Duration::from_secs(1));
        providers.claude_status(false).await;

        assert_eq!(probe.calls(), 3, "past its TTL a failed check is run again");
        assert_eq!(
            providers.model_source().rows(ACCOUNT).len(),
            CLAUDE_MODELS.len()
        );
    }

    /// A missing install, an old install and a sign-out each clear the rows, since
    /// the device no longer offers them.
    #[tokio::test]
    async fn a_missing_outdated_or_signed_out_claude_clears_the_rows() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        for (state, expected) in [
            (ProbeState::NotInstalled, ClaudeRuntimeState::NotInstalled),
            (ProbeState::Outdated, ClaudeRuntimeState::Outdated),
            (ProbeState::SignedOut, ClaudeRuntimeState::SignedOut),
        ] {
            probe.answer(ready_probe());
            providers.claude_status(true).await;
            assert_eq!(
                providers.model_source().rows(ACCOUNT).len(),
                CLAUDE_MODELS.len()
            );

            probe.answer(probe_answering(state));
            let status = providers.claude_status(true).await;

            assert_eq!(status.state, expected);
            assert!(providers.model_source().rows(ACCOUNT).is_empty());
        }
    }

    /// A caller that stops waiting does not lose the probe. The probe runs once, its
    /// answer is cached, and the next read is a cache hit.
    #[tokio::test]
    async fn a_caller_that_gives_up_leaves_its_probe_to_finish_and_be_cached() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let providers = PlanProviders::with_probe(
            root.path().to_path_buf(),
            secrets,
            gated_probe(
                Arc::clone(&started),
                Arc::clone(&release),
                ready_probe(),
                Arc::clone(&calls),
            ),
        );

        let gave_up =
            tokio::time::timeout(Duration::from_millis(20), providers.claude_status(false)).await;
        started.notified().await;
        release.notify_one();
        let status = providers.claude_status(false).await;

        assert!(
            gave_up.is_err(),
            "the probe was still running when the caller gave up"
        );
        assert_eq!(status.state, ClaudeRuntimeState::Ready);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the probe ran once");
        assert_eq!(
            providers.model_source().rows(ACCOUNT).len(),
            CLAUDE_MODELS.len()
        );
    }

    /// With no rows offered, a read waits for the probe, so the rows its answer gives
    /// are offered when the read returns.
    #[tokio::test]
    async fn a_read_with_no_rows_waits_for_the_probe_and_offers_its_rows() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let providers = PlanProviders::with_probe(
            root.path().to_path_buf(),
            secrets,
            gated_probe(
                Arc::clone(&started),
                Arc::clone(&release),
                ready_probe(),
                Arc::default(),
            ),
        );
        // Answers the probe once it is in flight, so the read is waiting when it does.
        let answering = tokio::spawn(async move {
            started.notified().await;
            release.notify_one();
        });

        providers.renew_claude_rows().await;

        assert_eq!(
            providers.model_source().rows(ACCOUNT).len(),
            CLAUDE_MODELS.len()
        );
        answering.await.expect("the probe was answered");
    }

    /// With rows offered and a fresh answer, a read does not wait for a refresh in
    /// flight. The refresh holds `refresh_lock` while it waits on the network, so the
    /// read returns at once and starts no probe.
    #[tokio::test]
    async fn a_read_with_rows_offered_does_not_wait_for_a_refresh_in_flight() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        providers.claude_status(false).await;
        assert_eq!(probe.calls(), 1);

        // The refresh in flight holds the lock for as long as this guard lives.
        let _refresh = providers.refresh_lock.lock().await;
        tokio::time::timeout(Duration::from_secs(1), providers.renew_claude_rows())
            .await
            .expect("the read did not wait for the refresh");

        assert_eq!(probe.calls(), 1, "no probe was started");
        assert_eq!(
            providers.model_source().rows(ACCOUNT).len(),
            CLAUDE_MODELS.len()
        );
    }

    /// Concurrent status reads share one probe. It runs alone, and the reads that
    /// arrive while it runs take its answer instead of starting another.
    #[tokio::test]
    async fn concurrent_status_reads_wait_for_the_probe_in_flight() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let probe: Probe = {
            let (running, peak, calls) =
                (Arc::clone(&running), Arc::clone(&peak), Arc::clone(&calls));
            Arc::new(move || -> ProbeFuture {
                let (running, peak, calls) =
                    (Arc::clone(&running), Arc::clone(&peak), Arc::clone(&calls));
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    running.fetch_sub(1, Ordering::SeqCst);
                    ready_probe()
                })
            })
        };
        let providers = PlanProviders::with_probe(root.path().to_path_buf(), secrets, probe);
        enable_claude(root.path());

        tokio::join!(
            providers.claude_status(true),
            providers.claude_status(true),
            providers.claude_status(true),
        );

        assert_eq!(peak.load(Ordering::SeqCst), 1, "probes must not overlap");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a waiting read reuses the probe"
        );
    }

    /// A probe that counts its runs and announces each start, then waits for `release`
    /// before it answers `answer`. The test decides when a probe is in flight and when
    /// it finishes.
    fn gated_probe(
        started: Arc<Notify>,
        release: Arc<Notify>,
        answer: ProbeResult,
        calls: Arc<AtomicUsize>,
    ) -> Probe {
        Arc::new(move || -> ProbeFuture {
            let (started, release, answer, calls) = (
                Arc::clone(&started),
                Arc::clone(&release),
                answer.clone(),
                Arc::clone(&calls),
            );
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                started.notify_one();
                release.notified().await;
                answer
            })
        })
    }

    /// A disable that lands while a probe is in flight is not undone when that
    /// probe answers `Ready`.
    #[tokio::test]
    async fn a_disable_during_the_probe_is_not_undone_by_its_answer() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let providers = Arc::new(PlanProviders::with_probe(
            root.path().to_path_buf(),
            secrets,
            gated_probe(
                Arc::clone(&started),
                Arc::clone(&release),
                ready_probe(),
                Arc::default(),
            ),
        ));

        let status = tokio::spawn({
            let providers = Arc::clone(&providers);
            async move { providers.claude_status(true).await }
        });
        started.notified().await;
        let off = providers.claude_set_enabled(false).await.expect("saved");
        release.notify_one();
        let answered = status.await.expect("status finished");

        assert_eq!(off.state, ClaudeRuntimeState::Disabled);
        assert_eq!(answered.state, ClaudeRuntimeState::Disabled);
        assert!(providers.model_source().rows(ACCOUNT).is_empty());
    }

    /// A disable waits for a refresh in flight. The refresh's probe answer is then
    /// written under the disable, so the disable is the last word.
    #[tokio::test]
    async fn a_disable_waits_for_a_refresh_in_flight() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let providers = Arc::new(PlanProviders::with_probe(
            root.path().to_path_buf(),
            secrets,
            gated_probe(
                Arc::clone(&started),
                Arc::clone(&release),
                ready_probe(),
                Arc::default(),
            ),
        ));
        let refreshed = Arc::new(AtomicBool::new(false));

        let refresh = tokio::spawn({
            let (providers, refreshed) = (Arc::clone(&providers), Arc::clone(&refreshed));
            async move {
                providers.refresh_models(&account()).await;
                refreshed.store(true, Ordering::SeqCst);
            }
        });
        // The refresh now holds `refresh_lock` while its probe is in flight.
        started.notified().await;
        let disable = tokio::spawn({
            let providers = Arc::clone(&providers);
            async move { providers.claude_set_enabled(false).await }
        });
        release.notify_one();
        let off = disable.await.expect("disable finished").expect("saved");

        assert!(
            refreshed.load(Ordering::SeqCst),
            "the disable ran before the refresh finished"
        );
        refresh.await.expect("refresh finished");
        assert_eq!(off.state, ClaudeRuntimeState::Disabled);
        assert!(providers.model_source().rows(ACCOUNT).is_empty());
    }

    /// A sign-out waits for a refresh in flight, so the refresh cannot write the
    /// account's rows back after the sign-out cleared them.
    #[tokio::test]
    async fn a_sign_out_waits_for_a_refresh_in_flight() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(300))
                    .set_body_json(json!({
                        "models": [{"slug": "gpt-6-luna", "display_name": "x", "visibility": "list"}]
                    })),
            )
            .mount(&server)
            .await;
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        seed_sign_in(root.path(), &secrets, &[PLAN_USAGE_SCOPE], true);
        let session = ChatgptPlanSession::new(
            PlanStore::new(root.path(), &account(), secrets.clone()),
            OAuthConfig::for_test(Url::parse(&server.uri()).expect("issuer")),
            Url::parse(&format!("{}/v1/models", server.uri())).expect("models"),
        )
        .expect("session");
        let probe = ScriptedProbe::new(ready_probe());
        let providers = Arc::new(providers_for(root.path(), &secrets, &probe));
        lock(&providers.sessions).insert(ACCOUNT.to_owned(), Arc::new(session));

        let refresh = tokio::spawn({
            let providers = Arc::clone(&providers);
            async move { providers.refresh_models(&account()).await }
        });
        // The refresh has sent its models request, so it holds `refresh_lock` and
        // its token has already been read.
        while server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let signed_out = providers.chatgpt_sign_out(&account()).await;
        refresh.await.expect("refresh finished");

        assert_ne!(signed_out.state, ChatgptPlanState::Ready);
        assert_eq!(providers.model_source().chatgpt_count(ACCOUNT), 0);
    }

    #[tokio::test]
    async fn turning_claude_on_persists_the_setting_and_reports_a_fresh_probe() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        let on = providers.claude_set_enabled(true).await.expect("saved");

        assert!(PlanSettingsStore::new(root.path()).load().claude_enabled);
        assert!(on.enabled);
        assert_eq!(on.state, ClaudeRuntimeState::Ready);
        assert_eq!(probe.calls(), 1);

        let off = providers.claude_set_enabled(false).await.expect("saved");

        assert!(!PlanSettingsStore::new(root.path()).load().claude_enabled);
        assert_eq!(off.state, ClaudeRuntimeState::Disabled);
        assert_eq!(probe.calls(), 1);
    }

    #[tokio::test]
    async fn the_status_view_carries_the_stored_effort_while_claude_is_off() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        PlanSettingsStore::new(root.path())
            .set_claude_effort(ClaudeEffort::Max)
            .expect("settings");

        let status = providers.claude_status(false).await;

        assert!(!status.enabled);
        assert_eq!(status.state, ClaudeRuntimeState::Disabled);
        assert_eq!(status.effort, ClaudeEffort::Max);
        assert_eq!(probe.calls(), 0);
    }

    #[tokio::test]
    async fn the_status_view_carries_the_stored_effort_when_claude_is_ready() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        PlanSettingsStore::new(root.path())
            .set_claude_effort(ClaudeEffort::Xhigh)
            .expect("settings");
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        let status = providers.claude_status(false).await;

        assert_eq!(status.state, ClaudeRuntimeState::Ready);
        assert_eq!(status.effort, ClaudeEffort::Xhigh);
    }

    /// The level is saved and reported with no probe, while Claude is off and on. The
    /// cached probe is reused when Claude is on, and the level is kept.
    #[tokio::test]
    async fn setting_the_effort_persists_it_and_probes_nothing() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        let off = providers
            .claude_set_effort(ClaudeEffort::Max)
            .await
            .expect("saved");

        assert!(!off.enabled);
        assert_eq!(off.state, ClaudeRuntimeState::Disabled);
        assert_eq!(off.effort, ClaudeEffort::Max);
        assert_eq!(
            PlanSettingsStore::new(root.path()).load().claude_effort,
            ClaudeEffort::Max
        );
        assert_eq!(probe.calls(), 0);

        providers.claude_set_enabled(true).await.expect("saved");
        assert_eq!(probe.calls(), 1);

        let on = providers
            .claude_set_effort(ClaudeEffort::Low)
            .await
            .expect("saved");

        assert!(on.enabled);
        assert_eq!(on.state, ClaudeRuntimeState::Ready);
        assert_eq!(on.effort, ClaudeEffort::Low);
        let stored = PlanSettingsStore::new(root.path()).load();
        assert!(stored.claude_enabled);
        assert_eq!(stored.claude_effort, ClaudeEffort::Low);
        assert_eq!(
            probe.calls(),
            1,
            "the cached probe is reused, not run again"
        );
    }

    #[tokio::test]
    async fn claude_is_refused_as_a_credential_while_off_and_granted_when_on() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        assert_eq!(
            providers
                .credential(ProviderId::ClaudeCode, &account())
                .err()
                .as_deref(),
            Some(CLAUDE_OFF_TEXT)
        );

        enable_claude(root.path());
        let credential = providers
            .credential(ProviderId::ClaudeCode, &account())
            .expect("credential");

        assert_eq!(credential.credential_id, "claude-account");
        assert!(credential.secret.is_empty());
        assert_eq!(credential.base_url, None);
    }

    #[tokio::test]
    async fn chatgpt_is_refused_as_a_credential_until_a_ready_sign_in_exists() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        assert_eq!(
            providers
                .credential(ProviderId::ChatgptPlan, &account())
                .err()
                .as_deref(),
            Some(CHATGPT_SIGN_IN_TEXT)
        );

        seed_sign_in(root.path(), &secrets, &[PLAN_USAGE_SCOPE], true);
        let credential = providers
            .credential(ProviderId::ChatgptPlan, &account())
            .expect("credential");
        assert_eq!(credential.credential_id, "chatgpt-plan");
        assert!(credential.secret.is_empty());
    }

    /// A sign-in whose key cannot be read is a failed check, not a sign-in that
    /// ended: the card says so, and the refusal does not ask for a new sign-in.
    #[tokio::test]
    async fn a_locked_sign_in_reads_as_check_failed_and_is_refused_without_a_new_sign_in() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        seed_sign_in(root.path(), &secrets, &[PLAN_USAGE_SCOPE], true);
        secrets.delete_account_keys(ACCOUNT).expect("delete keys");
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        let status = providers.chatgpt_status(Some(&account())).await;

        assert_eq!(status.state, ChatgptPlanState::CheckFailed);
        assert_eq!(
            providers
                .credential(ProviderId::ChatgptPlan, &account())
                .err()
                .as_deref(),
            Some(CHATGPT_CHECK_FAILED_TEXT)
        );
    }

    #[tokio::test]
    async fn a_sign_in_without_plan_usage_is_refused_as_a_credential() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        seed_sign_in(root.path(), &secrets, &["openid"], true);
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        assert_eq!(
            providers
                .credential(ProviderId::ChatgptPlan, &account())
                .err()
                .as_deref(),
            Some(CHATGPT_SIGN_IN_TEXT)
        );
    }

    #[tokio::test]
    async fn a_key_provider_is_refused_as_a_plan_credential() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        for provider in [
            ProviderId::OpenAi,
            ProviderId::OpenRouter,
            ProviderId::CustomOpenAi,
        ] {
            assert_eq!(
                providers.credential(provider, &account()).err().as_deref(),
                Some(NOT_A_PLAN_TEXT)
            );
        }
    }

    #[tokio::test]
    async fn the_claude_adapter_is_refused_while_off_and_built_when_on() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        assert_eq!(
            providers
                .adapter(ProviderId::ClaudeCode, &account(), None)
                .err(),
            Some(ProviderError::RuntimeUnavailable)
        );

        enable_claude(root.path());
        let adapter = providers
            .adapter(ProviderId::ClaudeCode, &account(), None)
            .expect("adapter");
        assert_eq!(adapter.provider_id(), ProviderId::ClaudeCode);
    }

    /// The first adapter built for a conversation keeps the level it was built
    /// with. A later adapter reuses it after the setting changes, and another
    /// conversation takes the setting that is current when it starts.
    #[tokio::test]
    async fn a_conversation_keeps_the_claude_level_it_started_with() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let settings = PlanSettingsStore::new(root.path());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        settings
            .set_claude_effort(ClaudeEffort::Low)
            .expect("settings");
        providers
            .adapter(ProviderId::ClaudeCode, &account(), Some("chat-a"))
            .expect("adapter");

        settings
            .set_claude_effort(ClaudeEffort::Max)
            .expect("settings");
        providers
            .adapter(ProviderId::ClaudeCode, &account(), Some("chat-a"))
            .expect("adapter");
        providers
            .adapter(ProviderId::ClaudeCode, &account(), Some("chat-b"))
            .expect("adapter");

        let store = ChatEffortStore::new(root.path(), &account());
        assert_eq!(
            store.lock_or_get("chat-a", ClaudeEffort::High),
            ClaudeEffort::Low
        );
        assert_eq!(
            store.lock_or_get("chat-b", ClaudeEffort::High),
            ClaudeEffort::Max
        );
    }

    /// Without a conversation, or with an id the store does not keep, the adapter
    /// is still built and nothing is written.
    #[tokio::test]
    async fn an_adapter_without_a_valid_conversation_stores_nothing() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        for conversation in [None, Some("has space")] {
            let adapter = providers
                .adapter(ProviderId::ClaudeCode, &account(), conversation)
                .expect("adapter");
            assert_eq!(adapter.provider_id(), ProviderId::ClaudeCode);
        }

        assert!(!chat_effort_file(root.path(), &account()).exists());
    }

    #[tokio::test]
    async fn a_claude_conversation_is_refused_while_off_and_stores_nothing() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        assert_eq!(
            providers
                .adapter(ProviderId::ClaudeCode, &account(), Some("chat-a"))
                .err(),
            Some(ProviderError::RuntimeUnavailable)
        );
        assert!(!chat_effort_file(root.path(), &account()).exists());
    }

    /// Each account keeps its own conversation levels, even when both use the same
    /// conversation id.
    #[tokio::test]
    async fn accounts_keep_their_conversation_levels_apart() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        enable_claude(root.path());
        let settings = PlanSettingsStore::new(root.path());
        let other = ValidatedAccountId::parse("person:other").expect("account");
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        settings
            .set_claude_effort(ClaudeEffort::Low)
            .expect("settings");
        providers
            .adapter(ProviderId::ClaudeCode, &account(), Some("chat-a"))
            .expect("adapter");
        settings
            .set_claude_effort(ClaudeEffort::Max)
            .expect("settings");
        providers
            .adapter(ProviderId::ClaudeCode, &other, Some("chat-a"))
            .expect("adapter");

        assert_eq!(
            ChatEffortStore::new(root.path(), &account()).lock_or_get("chat-a", ClaudeEffort::High),
            ClaudeEffort::Low
        );
        assert_eq!(
            ChatEffortStore::new(root.path(), &other).lock_or_get("chat-a", ClaudeEffort::High),
            ClaudeEffort::Max
        );
    }

    #[tokio::test]
    async fn the_chatgpt_adapter_is_built_and_a_key_provider_has_none() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        let adapter = providers
            .adapter(ProviderId::ChatgptPlan, &account(), None)
            .expect("adapter");
        assert_eq!(adapter.provider_id(), ProviderId::ChatgptPlan);

        assert_eq!(
            providers
                .adapter(ProviderId::Anthropic, &account(), None)
                .err(),
            Some(ProviderError::InvalidResponse)
        );
    }

    #[test]
    fn new_removes_stale_turn_directories_and_leaves_everything_else() {
        let root = tempfile::tempdir().expect("tempdir");
        let work = root.path().join(WORK_ROOT_DIR);
        fs::create_dir_all(work.join("turn-stale")).expect("stale turn");
        fs::write(work.join("turn-stale").join("prompt.txt"), "prompt").expect("prompt");
        fs::create_dir_all(work.join("keep-dir")).expect("other directory");
        fs::write(work.join("turn-file"), "not a directory").expect("turn-named file");

        let _providers = PlanProviders::new(
            root.path().to_path_buf(),
            Arc::new(InMemorySecretStore::default()),
        );

        assert!(!work.join("turn-stale").exists());
        assert!(work.join("keep-dir").is_dir());
        assert!(work.join("turn-file").is_file());
    }

    #[tokio::test]
    async fn a_sign_in_already_in_flight_is_reported_and_not_started_again() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        lock(&providers.sign_ins).insert(ACCOUNT.to_owned(), CancellationToken::new());

        let status = providers.chatgpt_sign_in(&account(), &NoBrowser).await;

        assert_eq!(status.state, ChatgptPlanState::SigningIn);
        assert!(status.error.is_none());
        assert!(lock(&providers.sign_ins).contains_key(ACCOUNT));
    }

    #[tokio::test]
    async fn cancelling_a_sign_in_fires_its_token() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        let token = CancellationToken::new();
        lock(&providers.sign_ins).insert(ACCOUNT.to_owned(), token.clone());

        providers.chatgpt_sign_in_cancel(&account());

        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn forgetting_the_account_cancels_sign_ins_and_clears_only_chatgpt_rows() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        let token = CancellationToken::new();
        lock(&providers.sign_ins).insert(ACCOUNT.to_owned(), token.clone());
        lock(&providers.sessions).insert(
            ACCOUNT.to_owned(),
            Arc::new(
                ChatgptPlanSession::production(PlanStore::new(
                    root.path(),
                    &account(),
                    secrets.clone(),
                ))
                .expect("session"),
            ),
        );
        providers.model_source().set_claude(claude_rows());
        providers
            .model_source()
            .set_chatgpt(ACCOUNT, vec![chatgpt_row("gpt-6-luna")]);

        providers.forget_account();

        assert!(token.is_cancelled());
        assert!(lock(&providers.sessions).is_empty());
        assert_eq!(providers.model_source().chatgpt_count(ACCOUNT), 0);
        assert_eq!(
            providers.model_source().rows(ACCOUNT).len(),
            CLAUDE_MODELS.len()
        );
    }

    #[tokio::test]
    async fn acknowledging_the_welcome_clears_the_pending_notice() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        seed_sign_in(root.path(), &secrets, &[PLAN_USAGE_SCOPE], false);
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        let before = providers.chatgpt_status(Some(&account())).await;
        let after = providers.chatgpt_welcome_ack(&account());

        assert_eq!(before.state, ChatgptPlanState::Ready);
        assert!(before.welcome_pending);
        assert!(!after.welcome_pending);
        assert_eq!(after.usage_url, CHATGPT_USAGE_URL);
    }

    #[tokio::test]
    async fn refreshing_with_no_chatgpt_sign_in_clears_its_rows_without_a_request() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        providers
            .model_source()
            .set_chatgpt(ACCOUNT, vec![chatgpt_row("gpt-6-luna")]);

        providers.refresh_models(&account()).await;

        assert_eq!(providers.model_source().chatgpt_count(ACCOUNT), 0);
        assert_eq!(probe.calls(), 0);
    }

    #[tokio::test]
    async fn signing_out_with_no_sign_in_reports_signed_out_without_an_error() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        providers
            .model_source()
            .set_chatgpt(ACCOUNT, vec![chatgpt_row("gpt-6-luna")]);

        let status = providers.chatgpt_sign_out(&account()).await;

        assert_eq!(status.state, ChatgptPlanState::SignedOut);
        assert_eq!(status.error, None);
        assert_eq!(providers.model_source().chatgpt_count(ACCOUNT), 0);
    }

    /// Tauri commands need `Send` futures, so the entry points are checked here
    /// without running them.
    #[test]
    fn the_async_entry_points_return_send_futures() {
        fn assert_send<F: Future + Send>(_future: F) {}

        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);
        let account = account();

        assert_send(providers.claude_status(true));
        assert_send(providers.claude_set_enabled(true));
        assert_send(providers.claude_set_effort(ClaudeEffort::High));
        assert_send(providers.chatgpt_status(Some(&account)));
        assert_send(providers.chatgpt_sign_in(&account, &NoBrowser));
        assert_send(providers.chatgpt_sign_out(&account));
        assert_send(providers.refresh_models(&account));
        assert_send(providers.renew_claude_rows());
        assert_send(providers.cached_claude_probe(true));
    }

    #[tokio::test]
    async fn a_signed_out_account_reports_signed_out_and_no_account_does_too() {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = Arc::new(InMemorySecretStore::default());
        let probe = ScriptedProbe::new(ready_probe());
        let providers = providers_for(root.path(), &secrets, &probe);

        let signed_out = providers.chatgpt_status(Some(&account())).await;
        let no_account = providers.chatgpt_status(None).await;

        assert_eq!(signed_out.state, ChatgptPlanState::SignedOut);
        assert_eq!(no_account.state, ChatgptPlanState::SignedOut);
        assert_eq!(no_account.model_count, 0);
    }
}
