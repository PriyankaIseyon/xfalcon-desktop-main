//! The company LLM gateway behind desktop private models.
//!
//! The known company profile is Qwen 3.6 35B on Ollama, behind Bifrost and a
//! Cloudflare Worker. Transport tuning applies only when both the host and
//! upstream model ID match exactly. Model settings come from discovery or user input.
//!
//! The model thinks before it answers, generates about 84 tokens per second,
//! and the engine serves one request at a time, so a request may wait in the
//! gateway's queue and then run for many minutes. The gateway's hard cap is 60
//! minutes per request, counted from submission. The timeouts, retry rule and
//! run limits below follow that, and none of them is shorter than the cap.

use std::time::Duration;

use reqwest::Url;

const HOST: &str = "llm-gateway.iseyon.workers.dev";
/// The `ollama/` prefix is required by the gateway.
pub(crate) const UPSTREAM_MODEL_ID: &str = "ollama/qwen3.6:35b-a3b-coding-nvfp4";
pub(crate) const DISPLAY_NAME: &str = "Qwen 3.6 35B Coding";

/// Total and per-read timeout. The gateway says to set no idle timeout below
/// 3,600 seconds, because a queued request receives no bytes.
pub(crate) const HTTP_TIMEOUT: Duration = Duration::from_secs(3_660);
/// Waits before the first, second and third retry. Only a connection failure,
/// HTTP 502 or HTTP 503 is retried, and only before any output has arrived.
pub(crate) const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(120),
];

/// How long the app sends the gateway whole (non-streamed) requests after one of
/// its streams was cut. The gateway has been seen closing streamed responses
/// within seconds while serving the same request whole, so after a cut the next
/// requests skip streaming instead of failing the same way again.
pub(crate) const NON_STREAMING_AFTER_CUT: Duration = Duration::from_secs(15 * 60);

/// The bound on one streamed response from the gateway. Its stream wraps every
/// token in about 700 bytes (the reasoning text three times, and routing
/// details), so a reply that uses the whole 32,768-token output budget is about
/// 23 MB, which the usual 16 MiB bound would call a malformed response. Nothing
/// is kept: each line is parsed and dropped, so this is a guard against a
/// stream that never ends, not a memory limit.
pub(crate) const STREAM_BYTE_LIMIT: usize = 128 * 1024 * 1024;

/// How much thinking text the model may produce before it has written any
/// answer. About 14,000 tokens, under three minutes of generation. The model
/// sometimes talks itself in circles on a trivial prompt until it has used its
/// whole 32,768-token budget, which is more than six minutes of nothing; past
/// this the turn ends and the run asks again with thinking off.
pub(crate) const MAX_THINKING_BYTES: usize = 48_000;

/// A run is many requests, each of which may wait and think for minutes.
pub(crate) const RUN_WALL_CLOCK: Duration = Duration::from_secs(2 * 60 * 60);
/// Longer than [`HTTP_TIMEOUT`], so a request that is queued or being retried
/// is never reported as a stalled run.
pub(crate) const RUN_STALL_LIMIT: Duration = Duration::from_secs(3_720);
/// Backstop for the wall clock, like the default run limit's extra minute.
pub(crate) const RUN_HARD_LIMIT: Duration = Duration::from_secs(2 * 60 * 60 + 60);

/// Whether `base_url` points at the company gateway. Only `https` on the default
/// port: the same host over `http` or on another port is an ordinary private endpoint.
pub(crate) fn is_gateway_base_url(base_url: &str) -> bool {
    Url::parse(base_url).ok().is_some_and(|url| {
        url.scheme() == "https" && url.port().is_none() && url.host_str() == Some(HOST)
    })
}

/// Whether this is the one company-gateway model with the tuned transport and
/// inference profile. Other model IDs may share the host without inheriting it.
pub(crate) fn is_gateway_model(base_url: &str, upstream_model_id: &str) -> bool {
    is_gateway_base_url(base_url) && upstream_model_id == UPSTREAM_MODEL_ID
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_gateway_host_matches() {
        assert!(is_gateway_base_url(
            "https://llm-gateway.iseyon.workers.dev/v1"
        ));
        assert!(is_gateway_base_url(
            "https://llm-gateway.iseyon.workers.dev"
        ));
        for other in [
            "https://api.example.org/v1",
            "https://llm-gateway.iseyon.workers.dev.evil.example/v1",
            "https://evil.example/llm-gateway.iseyon.workers.dev",
            "not a url",
        ] {
            assert!(!is_gateway_base_url(other), "{other}");
        }
    }

    #[test]
    fn the_gateway_profile_needs_https_on_the_default_port() {
        assert!(is_gateway_base_url(
            "https://llm-gateway.iseyon.workers.dev/v1"
        ));
        assert!(!is_gateway_base_url(
            "http://llm-gateway.iseyon.workers.dev/v1"
        ));
        assert!(!is_gateway_base_url(
            "https://llm-gateway.iseyon.workers.dev:8443/v1"
        ));
        assert!(is_gateway_base_url(
            "https://llm-gateway.iseyon.workers.dev:443/v1"
        ));
    }

    #[test]
    fn the_gateway_profile_requires_the_exact_model_id() {
        let base_url = "https://llm-gateway.iseyon.workers.dev/v1";
        assert!(is_gateway_model(base_url, UPSTREAM_MODEL_ID));
        assert!(!is_gateway_model(base_url, "organization/another-model"));
    }

    #[test]
    fn run_limits_outlast_a_queued_request() {
        assert!(RUN_STALL_LIMIT > HTTP_TIMEOUT);
        assert!(RUN_HARD_LIMIT > RUN_WALL_CLOCK);
    }
}
