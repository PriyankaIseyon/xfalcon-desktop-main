//! Bounded presentation stream with delta coalescing.

use std::time::Duration;

use tokio::time::Instant;
use xfalcon_contracts::inference::{InferenceEvent, ReasoningSummary};
use xfalcon_contracts::RunId;

/// Maximum text or reasoning delta admitted to the presentation layer.
///
/// Provider transport lines have a larger independent ceiling because one
/// opaque compaction block may arrive on a single SSE line. That must not
/// widen frontend event payloads.
const MAX_DELTA_BYTES: usize = 256 * 1024;

/// Hard ceiling on one forwarded reasoning event, from the contract.
///
/// `ReasoningSummary::provider_designated` refuses anything larger, and it is
/// the only constructor. Kept here as a named constant because this module has
/// to stay UNDER it while coalescing: a buffer that assembled more than this
/// could not be forwarded at all, and the previous code's response to a refusal
/// was `.ok()` — the fragment vanished with nothing said.
const MAX_REASONING_EVENT_BYTES: usize = 4 * 1024;

/// The two bounds on how long buffered text may be held.
///
/// Coalescing exists because a provider emits one delta per token and each
/// forwarded delta costs a JSON serialization and an IPC hop to the webview.
/// Holding text is only ever a means to that end: whichever bound is reached
/// first, the text goes out. A config that holds text until the turn ends is not
/// a coalescer, it is a buffer, and a buffer is what makes an answer arrive all
/// at once.
#[derive(Clone, Copy, Debug)]
pub struct CoalesceConfig {
    /// Longest the sink may hold buffered text. `Duration::ZERO` means forward
    /// every delta as it arrives.
    ///
    /// The sink can only observe this bound when the next delta arrives, so a
    /// caller that wants it to hold while the provider is quiet drives
    /// [`InferencePresentationSink::flush`] on its own timer at this period.
    pub min_coalesce: Duration,
    /// Buffered bytes that force a forward regardless of the interval.
    ///
    /// This is what keeps a provider that outruns the caller's timer painting
    /// progressively. The bound is checked after the incoming delta is appended,
    /// so one forwarded delta carries at most this many bytes plus one incoming
    /// delta — both bounded, `MAX_DELTA_BYTES` being the second.
    pub max_coalesce_bytes: usize,
}

impl CoalesceConfig {
    /// Forward every delta, so a test asserts on the events it emitted rather
    /// than on a clock.
    #[must_use]
    pub fn for_tests() -> Self {
        Self {
            min_coalesce: Duration::ZERO,
            max_coalesce_bytes: 4 * 1024,
        }
    }

    /// The production cadence: roughly one forward per animation frame.
    ///
    /// 16 ms matches the frontend's own rAF commit coalescing
    /// (`chatStore.setStateCoalesced`), so a shorter interval would buy a
    /// forward the reader cannot see and a longer one would drop frames the
    /// reader can. 4 KiB is a few lines of prose — small enough that growth
    /// still reads as continuous, large enough that a fast provider does not
    /// produce one forward per token.
    #[must_use]
    pub fn for_delta_coalescing() -> Self {
        Self {
            min_coalesce: Duration::from_millis(16),
            max_coalesce_bytes: 4 * 1024,
        }
    }

    /// The byte bound for buffered reasoning, which is the text bound capped so
    /// one assembled event still fits [`MAX_REASONING_EVENT_BYTES`].
    fn reasoning_bytes(self) -> usize {
        self.max_coalesce_bytes.min(MAX_REASONING_EVENT_BYTES / 2)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamError {
    OversizedPayload,
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OversizedPayload => write!(f, "inference payload exceeds limit"),
        }
    }
}

impl std::error::Error for StreamError {}

/// One run of buffered text of a single kind.
///
/// Two of these rather than one shared buffer, because answer text and
/// reasoning are two streams that land in two places: the frontend appends text
/// to the answer and files reasoning against the tool rows drawn so far
/// (`chatReduce`), on two independent sequence ledgers. Their order relative to
/// EACH OTHER is therefore not observable; their order relative to any other
/// event is, which is why both are flushed before one is forwarded.
struct Pending {
    text: String,
    /// Sequence of the FIRST delta in the buffer.
    sequence: u64,
    /// When the buffered text began.
    since: Instant,
}

/// Coalesces a run's text and reasoning deltas on the way to the presentation
/// callback.
///
/// Sequence numbers come from the FIRST delta of each buffered batch. The
/// frontend admits a delta when its sequence exceeds the highest it has seen
/// (`chatReduce.reduceInferenceEvent`), and first-sequences increase from one
/// batch to the next, so skipping the sequences in between is correct rather
/// than merely tolerated.
pub struct InferencePresentationSink<F> {
    run_id: RunId,
    config: CoalesceConfig,
    pending_text: Option<Pending>,
    pending_reasoning: Option<Pending>,
    collected: Vec<InferenceEvent>,
    on_inference: F,
}

impl<F> InferencePresentationSink<F>
where
    F: FnMut(&InferenceEvent),
{
    #[must_use]
    pub fn new(run_id: RunId, config: CoalesceConfig, on_inference: F) -> Self {
        Self {
            run_id,
            config,
            pending_text: None,
            pending_reasoning: None,
            collected: Vec::new(),
            on_inference,
        }
    }

    /// The period a caller should drive [`Self::flush`] at.
    ///
    /// The sink owns the cadence, so the caller's timer reads it here rather
    /// than holding a second copy that can drift from it.
    #[must_use]
    pub fn coalesce_interval(&self) -> Duration {
        self.config.min_coalesce
    }

    pub fn emit(&mut self, event: InferenceEvent) -> Result<(), StreamError> {
        match event {
            InferenceEvent::TextDelta { text, sequence, .. } => {
                if text.len() > MAX_DELTA_BYTES {
                    return Err(StreamError::OversizedPayload);
                }
                let bound = self.config.max_coalesce_bytes;
                Self::append(&mut self.pending_text, &text, sequence);
                if Self::meets_a_bound(&self.pending_text, bound, self.config.min_coalesce) {
                    self.flush_text();
                }
            }
            // Every other event ends the current runs of buffered prose: the
            // buffered text was produced before it, and forwarding it afterwards
            // would reorder the answer around a tool call or a terminal event.
            other => {
                self.flush();
                self.forward(other);
            }
        }
        Ok(())
    }

    /// Buffers one reasoning fragment.
    ///
    /// Raw text rather than a built [`InferenceEvent`], because the event's
    /// payload type is opaque and capped: assembling several fragments means
    /// concatenating before construction, not after. It arrives at the same
    /// token cadence as answer text and used to be forwarded one IPC hop per
    /// fragment, which became the run's dominant presentation cost the moment
    /// reasoning was requested from every shipped vendor.
    pub fn emit_reasoning(&mut self, text: &str, sequence: u64) -> Result<(), StreamError> {
        if text.len() > MAX_DELTA_BYTES {
            return Err(StreamError::OversizedPayload);
        }
        if text.is_empty() {
            return Ok(());
        }
        // Flushed BEFORE the append when the fragment would take the buffer past
        // what one event may carry, so the assembled text always fits its
        // constructor and no fragment is ever dropped for being late in a batch.
        if let Some(pending) = &self.pending_reasoning {
            if pending.text.len() + text.len() > MAX_REASONING_EVENT_BYTES {
                self.flush_reasoning();
            }
        }
        let bound = self.config.reasoning_bytes();
        Self::append(&mut self.pending_reasoning, text, sequence);
        if Self::meets_a_bound(&self.pending_reasoning, bound, self.config.min_coalesce) {
            self.flush_reasoning();
        }
        Ok(())
    }

    /// Forwards whatever text is buffered, of either kind.
    ///
    /// The caller drives this on a timer, which is what makes `min_coalesce` a
    /// bound rather than a hint: a provider that sends three tokens and then
    /// goes quiet has already produced text the reader should see, and the sink
    /// on its own cannot know that time has passed until the next delta.
    pub fn flush(&mut self) {
        self.flush_text();
        self.flush_reasoning();
    }

    pub fn finish(mut self) -> Vec<InferenceEvent> {
        self.flush();
        self.collected
    }

    fn flush_text(&mut self) {
        let Some(pending) = self.pending_text.take() else {
            return;
        };
        let event = InferenceEvent::text_delta(self.run_id.clone(), pending.sequence, pending.text);
        self.forward(event);
    }

    fn flush_reasoning(&mut self) {
        let Some(pending) = self.pending_reasoning.take() else {
            return;
        };
        // `emit_reasoning` holds the buffer under the cap, so the only way this
        // refuses is a single fragment larger than one event may carry — which
        // no provider streaming reasoning token by token produces. Truncating at
        // a character boundary keeps what the reader can use and says so;
        // `.ok()` here is what silently deleted a fragment before.
        let text = match ReasoningSummary::provider_designated(pending.text.clone()) {
            Ok(summary) => summary,
            Err(error) => {
                tracing::warn!(%error, "reasoning fragment truncated to the event bound");
                let mut cut = MAX_REASONING_EVENT_BYTES;
                while cut > 0 && !pending.text.is_char_boundary(cut) {
                    cut -= 1;
                }
                match ReasoningSummary::provider_designated(pending.text[..cut].to_owned()) {
                    Ok(summary) => summary,
                    // Unreachable: the slice is at most the bound. Dropping is
                    // still better than panicking on a presentation payload.
                    Err(_) => return,
                }
            }
        };
        let event = InferenceEvent::reasoning_summary(self.run_id.clone(), pending.sequence, text);
        self.forward(event);
    }

    fn append(slot: &mut Option<Pending>, text: &str, sequence: u64) {
        match slot {
            Some(pending) => pending.text.push_str(text),
            None => {
                if text.is_empty() {
                    // An empty delta must not start the clock: a timer that
                    // outlives its text is a bound measured from the wrong
                    // moment.
                    return;
                }
                *slot = Some(Pending {
                    text: text.to_owned(),
                    sequence,
                    since: Instant::now(),
                });
            }
        }
    }

    /// Whether the buffered text has met either bound.
    ///
    /// A zero interval reports true for the first delta, which is how
    /// [`CoalesceConfig::for_tests`] gets one forward per delta without a
    /// separate branch.
    fn meets_a_bound(slot: &Option<Pending>, max_bytes: usize, min_coalesce: Duration) -> bool {
        let Some(pending) = slot else {
            return false;
        };
        pending.text.len() >= max_bytes || pending.since.elapsed() >= min_coalesce
    }

    fn forward(&mut self, event: InferenceEvent) {
        (self.on_inference)(&event);
        self.collected.push(event);
    }
}
