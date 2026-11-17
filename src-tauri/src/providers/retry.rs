//! Retries a provider turn that failed transiently, before it produced anything.
//!
//! # Why a decorator rather than a loop inside each adapter
//!
//! All three adapters have the same shape — build the request, send, classify a
//! non-success status, stream SSE — so all three would grow the same loop. More
//! importantly, the condition that makes a retry *safe* is not visible inside an
//! adapter: it is whether the caller has already been handed part of an answer.
//! Wrapping [`ProviderAdapter`] puts the retry exactly where that is knowable,
//! and keeps every adapter a straight-line description of one vendor's wire
//! format.
//!
//! # The safety rule
//!
//! A retry re-sends the whole turn, so the model answers from the beginning. If
//! output already reached the caller, those bytes are in the transcript and
//! on screen, and a second attempt would append a second answer to the first.
//! So **a turn that has emitted output is never retried**, however transient
//! the failure looks. Internal `Progress` events carry no output and do not
//! prevent a retry. In practice this is not much
//! of a restriction: rate limits and overload arrive as a non-success status
//! before the body streams at all, which is precisely the case worth retrying.
//!
//! # What is retried
//!
//! [`ProviderError::RateLimited`] and [`ProviderError::Unavailable`] — the two
//! that say "the same request might work shortly". Everything else is
//! deterministic and is reported on the first attempt:
//! [`ProviderError::AuthenticationFailed`] (a bad key stays bad, and hammering
//! it risks the provider locking the account),
//! [`ProviderError::PaymentRequired`] (the key is fine and the account is not),
//! [`ProviderError::PayloadTooLarge`] (the same bytes are refused every time),
//! [`ProviderError::TlsVerificationFailed`] (the next handshake presents the
//! same chain to the same trust store), [`ProviderError::InvalidResponse`] (the
//! request or the parse is wrong, and repeating it changes nothing), and
//! [`ProviderError::Cancelled`] (the user asked us to stop).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use xfalcon_contracts::credentials::ProviderId;

use super::adapter::{ProviderAdapter, ProviderChatRequest, ProviderError, ProviderStreamEvent};

/// How many times one turn is attempted in total, the first try included.
///
/// Three is the point past which waiting stops being better than telling the
/// user. A provider still rate-limiting after two backoffs is not having a
/// blip, and the run can be retried from the UI — which is a decision the
/// person watching the spinner should get to make.
const MAX_ATTEMPTS: u32 = 3;

/// Wait before the second attempt; doubled before the third.
const BASE_DELAY: Duration = Duration::from_millis(500);

/// Ceiling on any single wait, and on the `Retry-After` this will honour.
///
/// A provider asking for longer than this is not describing a blip either.
/// Reporting the rate limit immediately beats holding an interactive turn
/// open for a minute with no way for the user to tell the difference between
/// "waiting" and "hung".
const MAX_DELAY: Duration = Duration::from_secs(15);

/// Wraps an adapter so transient failures are retried while it is still safe to.
pub(crate) struct RetryingAdapter {
    inner: Arc<dyn ProviderAdapter>,
}

impl RetryingAdapter {
    #[must_use]
    pub(crate) fn new(inner: Arc<dyn ProviderAdapter>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl ProviderAdapter for RetryingAdapter {
    fn provider_id(&self) -> ProviderId {
        self.inner.provider_id()
    }

    async fn probe_native_compaction(
        &self,
        model_id: &str,
        route_identity: &str,
        session_id: &str,
        secret: &str,
        cancel: &CancellationToken,
    ) -> Result<(), ProviderError> {
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(ProviderError::Cancelled),
            result = self.inner.probe_native_compaction(
                model_id, route_identity, session_id, secret, cancel,
            ) => result,
        }
    }

    async fn stream(
        &self,
        request: &ProviderChatRequest,
        secret: &str,
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
    ) -> Result<(), ProviderError> {
        for attempt in 0..MAX_ATTEMPTS {
            let mut emitted = false;
            let outcome = {
                let mut observing = |event: ProviderStreamEvent| {
                    emitted |= !matches!(event, ProviderStreamEvent::Progress);
                    on_event(event);
                };
                // Adapters observe cancellation while reading SSE, but the
                // HTTP send can still be waiting for response headers. Own
                // cancellation around the entire attempt for every provider.
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => return Err(ProviderError::Cancelled),
                    result = self.inner.stream(request, secret, cancel, &mut observing) => result,
                }
            };

            let Err(error) = outcome else {
                return Ok(());
            };
            if emitted || attempt + 1 == MAX_ATTEMPTS {
                return Err(error);
            }
            let Some(delay) = delay_before_retry(error, attempt) else {
                return Err(error);
            };

            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(ProviderError::Cancelled),
                () = tokio::time::sleep(delay) => {}
            }
        }
        // `MAX_ATTEMPTS` is a non-zero constant, so the loop always returns.
        Err(ProviderError::Unavailable)
    }
}

/// How long to wait before re-attempting, or `None` when `error` must not be
/// retried at all.
///
/// A `Retry-After` the provider actually sent wins over local backoff — it is
/// the only party that knows when the limit lifts — but only up to
/// [`MAX_DELAY`]; past that the wait is reported rather than served.
fn delay_before_retry(error: ProviderError, attempt: u32) -> Option<Duration> {
    let backoff = BASE_DELAY.saturating_mul(1 << attempt);
    match error {
        ProviderError::RateLimited {
            retry_after: Some(requested),
        } => (requested <= MAX_DELAY).then_some(requested),
        // A timed-out request was retried as `Unavailable` before the two were
        // told apart, and nothing has reached the caller yet, so it still is.
        ProviderError::RateLimited { retry_after: None }
        | ProviderError::Unavailable
        | ProviderError::TimedOut => Some(backoff.min(MAX_DELAY)),
        // Deterministic failures. Each already carries its own remedy — top up
        // the account, send less, fix the certificate chain — and none of those
        // remedies is "wait", so a retry only delays the true answer and then
        // reports the loop's verdict instead of the real cause.
        ProviderError::PaymentRequired
        | ProviderError::CustomAuthenticationFailed
        | ProviderError::EndpointUnavailable
        | ProviderError::AddressRefused
        | ProviderError::PayloadTooLarge
        | ProviderError::NativeCompactionUnsupported
        | ProviderError::MalformedCompaction
        | ProviderError::ContextOverflow
        | ProviderError::TlsVerificationFailed
        | ProviderError::AuthenticationFailed
        | ProviderError::InvalidResponse
        | ProviderError::PlanSignInRequired
        | ProviderError::PlanUsageLimit
        | ProviderError::PlanNotEligible
        | ProviderError::RuntimeUnavailable
        | ProviderError::Cancelled => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    type Attempt = (Vec<ProviderStreamEvent>, Result<(), ProviderError>);

    /// Answers with a scripted outcome per attempt, recording how many it saw.
    struct ScriptedAdapter {
        outcomes: Mutex<Vec<Attempt>>,
        attempts: Mutex<u32>,
    }

    impl ScriptedAdapter {
        fn new(outcomes: Vec<Result<Vec<ProviderStreamEvent>, ProviderError>>) -> Arc<Self> {
            Self::with_attempts(
                outcomes
                    .into_iter()
                    .map(|outcome| match outcome {
                        Ok(events) => (events, Ok(())),
                        Err(error) => (Vec::new(), Err(error)),
                    })
                    .collect(),
            )
        }

        fn with_attempts(outcomes: Vec<Attempt>) -> Arc<Self> {
            Arc::new(Self {
                outcomes: Mutex::new(outcomes),
                attempts: Mutex::new(0),
            })
        }

        fn attempts(&self) -> u32 {
            *self.attempts.lock().expect("lock")
        }
    }

    #[async_trait]
    impl ProviderAdapter for ScriptedAdapter {
        fn provider_id(&self) -> ProviderId {
            ProviderId::Anthropic
        }

        async fn stream(
            &self,
            _request: &ProviderChatRequest,
            _secret: &str,
            _cancel: &CancellationToken,
            on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
        ) -> Result<(), ProviderError> {
            *self.attempts.lock().expect("lock") += 1;
            let scripted = {
                let mut outcomes = self.outcomes.lock().expect("lock");
                if outcomes.is_empty() {
                    (Vec::new(), Ok(()))
                } else {
                    outcomes.remove(0)
                }
            };
            for event in scripted.0 {
                on_event(event);
            }
            scripted.1
        }
    }

    fn request() -> ProviderChatRequest {
        ProviderChatRequest {
            model_id: "test-model".to_owned(),
            messages: Vec::new(),
            max_output_tokens: Some(128),
            allow_reasoning: true,
            tools: Vec::new(),
            openrouter: None,
        }
    }

    async fn run(
        inner: Arc<ScriptedAdapter>,
    ) -> (Result<(), ProviderError>, Vec<ProviderStreamEvent>) {
        let seen = Mutex::new(Vec::new());
        let outcome = {
            let mut collect = |event: ProviderStreamEvent| seen.lock().expect("lock").push(event);
            RetryingAdapter::new(inner)
                .stream(
                    &request(),
                    "secret",
                    &CancellationToken::new(),
                    &mut collect,
                )
                .await
        };
        (outcome, seen.into_inner().expect("lock"))
    }

    #[tokio::test(start_paused = true)]
    async fn a_rate_limit_is_retried_and_the_second_attempt_answers() {
        let inner = ScriptedAdapter::new(vec![
            Err(ProviderError::RateLimited { retry_after: None }),
            Ok(vec![ProviderStreamEvent::TextDelta("hello".to_owned())]),
        ]);

        let (outcome, events) = run(inner.clone()).await;

        assert_eq!(outcome, Ok(()));
        assert_eq!(inner.attempts(), 2);
        assert_eq!(events.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn partial_argument_progress_allows_a_safe_transient_retry() {
        let call = ProviderStreamEvent::ToolCallRequested {
            call_id: "edit-both".to_owned(),
            name: "project_propose_patch".to_owned(),
            arguments: r#"{"operations":[]}"#.to_owned(),
        };
        let inner = ScriptedAdapter::with_attempts(vec![
            (
                vec![ProviderStreamEvent::Progress],
                Err(ProviderError::Unavailable),
            ),
            (
                vec![
                    ProviderStreamEvent::Progress,
                    call.clone(),
                    ProviderStreamEvent::Completed,
                ],
                Ok(()),
            ),
        ]);
        let (outcome, events) = run(inner.clone()).await;
        assert_eq!(outcome, Ok(()));
        assert_eq!(inner.attempts(), 2);
        assert_eq!(
            events,
            vec![
                ProviderStreamEvent::Progress,
                ProviderStreamEvent::Progress,
                call,
                ProviderStreamEvent::Completed
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_complete_tool_call_prevents_retry_even_after_argument_progress() {
        let call = ProviderStreamEvent::ToolCallRequested {
            call_id: "edit-both".to_owned(),
            name: "project_propose_patch".to_owned(),
            arguments: r#"{"operations":[]}"#.to_owned(),
        };
        let inner = ScriptedAdapter::with_attempts(vec![
            (
                vec![ProviderStreamEvent::Progress, call.clone()],
                Err(ProviderError::Unavailable),
            ),
            (vec![call.clone(), ProviderStreamEvent::Completed], Ok(())),
        ]);
        let (outcome, events) = run(inner.clone()).await;
        assert_eq!(outcome, Err(ProviderError::Unavailable));
        assert_eq!(inner.attempts(), 1);
        assert_eq!(events, vec![ProviderStreamEvent::Progress, call]);
    }

    #[tokio::test(start_paused = true)]
    async fn attempts_stop_at_the_ceiling_and_report_the_last_error() {
        let inner = ScriptedAdapter::new(vec![
            Err(ProviderError::Unavailable),
            Err(ProviderError::Unavailable),
            Err(ProviderError::RateLimited { retry_after: None }),
        ]);

        let (outcome, _) = run(inner.clone()).await;

        assert_eq!(
            outcome,
            Err(ProviderError::RateLimited { retry_after: None })
        );
        assert_eq!(inner.attempts(), MAX_ATTEMPTS);
    }

    /// The safety rule: once the caller has part of an answer, a retry would
    /// append a second answer to the first.
    #[tokio::test(start_paused = true)]
    async fn a_failure_after_output_has_started_is_never_retried() {
        let inner = ScriptedAdapter::new(vec![Err(ProviderError::Unavailable)]);
        let seen = Mutex::new(Vec::new());

        // A stream that emits and *then* fails, which the scripted adapter
        // cannot express on its own.
        struct EmitThenFail;
        #[async_trait]
        impl ProviderAdapter for EmitThenFail {
            fn provider_id(&self) -> ProviderId {
                ProviderId::Anthropic
            }
            async fn stream(
                &self,
                _request: &ProviderChatRequest,
                _secret: &str,
                _cancel: &CancellationToken,
                on_event: &mut (dyn FnMut(ProviderStreamEvent) + Send),
            ) -> Result<(), ProviderError> {
                on_event(ProviderStreamEvent::TextDelta("half an ans".to_owned()));
                Err(ProviderError::Unavailable)
            }
        }
        drop(inner);

        let outcome = {
            let mut collect = |event: ProviderStreamEvent| seen.lock().expect("lock").push(event);
            RetryingAdapter::new(Arc::new(EmitThenFail))
                .stream(
                    &request(),
                    "secret",
                    &CancellationToken::new(),
                    &mut collect,
                )
                .await
        };

        assert_eq!(outcome, Err(ProviderError::Unavailable));
        assert_eq!(
            seen.into_inner().expect("lock").len(),
            1,
            "the partial answer must reach the caller exactly once"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_bad_credential_is_reported_immediately() {
        let inner = ScriptedAdapter::new(vec![Err(ProviderError::AuthenticationFailed)]);

        let (outcome, _) = run(inner.clone()).await;

        assert_eq!(outcome, Err(ProviderError::AuthenticationFailed));
        assert_eq!(
            inner.attempts(),
            1,
            "retrying a rejected key risks the provider locking the account"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancelled_turn_is_not_retried() {
        let inner = ScriptedAdapter::new(vec![Err(ProviderError::Cancelled)]);

        let (outcome, _) = run(inner.clone()).await;

        assert_eq!(outcome, Err(ProviderError::Cancelled));
        assert_eq!(inner.attempts(), 1);
    }

    #[test]
    fn a_served_retry_after_wins_over_local_backoff() {
        assert_eq!(
            delay_before_retry(
                ProviderError::RateLimited {
                    retry_after: Some(Duration::from_secs(9))
                },
                0
            ),
            Some(Duration::from_secs(9))
        );
    }

    #[test]
    fn a_retry_after_beyond_the_ceiling_is_reported_rather_than_waited_out() {
        assert_eq!(
            delay_before_retry(
                ProviderError::RateLimited {
                    retry_after: Some(MAX_DELAY + Duration::from_secs(1))
                },
                0
            ),
            None
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_deterministic_failure_is_attempted_exactly_once() {
        // The whole point of naming these apart from `Unavailable`: none of
        // them changes by being asked again, so waiting only delays the true
        // answer and then replaces it with the retry loop's verdict.
        for error in [
            ProviderError::PaymentRequired,
            ProviderError::CustomAuthenticationFailed,
            ProviderError::EndpointUnavailable,
            ProviderError::AddressRefused,
            ProviderError::PayloadTooLarge,
            ProviderError::TlsVerificationFailed,
        ] {
            let inner = ScriptedAdapter::new(vec![Err(error)]);

            let (outcome, _) = run(inner.clone()).await;

            assert_eq!(outcome, Err(error));
            assert_eq!(inner.attempts(), 1, "{error} must not be retried");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn naming_deterministic_failures_did_not_stop_the_transient_ones_retrying() {
        for error in [
            ProviderError::RateLimited { retry_after: None },
            ProviderError::Unavailable,
        ] {
            let inner = ScriptedAdapter::new(vec![Err(error), Err(error), Err(error)]);

            let (_, _) = run(inner.clone()).await;

            assert_eq!(
                inner.attempts(),
                MAX_ATTEMPTS,
                "{error} is worth waiting for"
            );
        }
    }

    #[test]
    fn backoff_grows_and_then_holds_at_the_ceiling() {
        assert_eq!(
            delay_before_retry(ProviderError::Unavailable, 0),
            Some(BASE_DELAY)
        );
        assert_eq!(
            delay_before_retry(ProviderError::Unavailable, 1),
            Some(BASE_DELAY * 2)
        );
        assert_eq!(
            delay_before_retry(ProviderError::Unavailable, 20),
            Some(MAX_DELAY)
        );
    }
}
