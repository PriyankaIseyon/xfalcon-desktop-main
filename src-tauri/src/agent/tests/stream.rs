use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use xfalcon_contracts::credentials::{ModelId, ProviderId};
use xfalcon_contracts::inference::{FinishReason, InferenceEvent};
use xfalcon_contracts::{RunId, CONTRACT_VERSION};

use crate::agent::stream::{CoalesceConfig, InferencePresentationSink, StreamError};

fn run_id() -> RunId {
    RunId::from("run-stream-1")
}

/// A forwarding target the test can read WHILE the sink is still alive.
///
/// That is the whole point: a sink that only ever forwards at `finish` looks
/// identical to a correct one if the assertion runs after `finish`. Every test
/// below that cares about when text was forwarded reads this mid-run.
type Forwarded = Rc<RefCell<Vec<InferenceEvent>>>;

fn recorder() -> (Forwarded, impl FnMut(&InferenceEvent)) {
    let seen: Forwarded = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&seen);
    (seen, move |event: &InferenceEvent| {
        sink.borrow_mut().push(event.clone());
    })
}

/// Reasoning is opaque once constructed — `ReasoningSummary` has no accessor —
/// so what a test can read is the SHAPE of the serialized event. That is enough
/// for the property that matters: how many reasoning events were forwarded, and
/// what text each carried.
fn reasonings(events: &[InferenceEvent]) -> Vec<String> {
    events
        .iter()
        .filter(|event| matches!(event, InferenceEvent::ReasoningSummary { .. }))
        .map(|event| {
            let value = serde_json::to_value(event).expect("serializes");
            value["summary"]
                .as_str()
                .expect("summary string")
                .to_owned()
        })
        .collect()
}

fn texts(events: &[InferenceEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            InferenceEvent::TextDelta { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Adjacent deltas inside one interval become one forward. This is the half of
/// the old contract that was right, kept.
#[tokio::test(start_paused = true)]
async fn coalesces_adjacent_deltas_inside_one_interval() {
    let run = run_id();
    let (seen, on_event) = recorder();
    let config = CoalesceConfig {
        min_coalesce: Duration::from_secs(5),
        max_coalesce_bytes: 4 * 1024,
    };
    let mut sink = InferencePresentationSink::new(run.clone(), config, on_event);

    sink.emit(InferenceEvent::text_delta(run.clone(), 1, "hel"))
        .expect("delta 1");
    sink.emit(InferenceEvent::text_delta(run.clone(), 2, "lo"))
        .expect("delta 2");
    assert!(
        seen.borrow().is_empty(),
        "neither bound was met, so nothing should have been forwarded yet"
    );

    let collected = sink.finish();
    assert_eq!(texts(&collected), vec!["hello".to_owned()]);
    assert_eq!(texts(&seen.borrow()), vec!["hello".to_owned()]);
}

/// The defect this change exists to fix. The previous version of this test
/// asserted `forwarded.len() == 1` for a whole turn, which is what let a
/// production answer arrive in one piece at the end.
#[tokio::test(start_paused = true)]
async fn forwards_buffered_text_once_the_interval_has_elapsed() {
    let run = run_id();
    let (seen, on_event) = recorder();
    let mut sink = InferencePresentationSink::new(
        run.clone(),
        CoalesceConfig::for_delta_coalescing(),
        on_event,
    );

    sink.emit(InferenceEvent::text_delta(run.clone(), 1, "first "))
        .expect("delta 1");
    assert!(seen.borrow().is_empty(), "the interval has not elapsed yet");

    tokio::time::advance(Duration::from_millis(20)).await;
    sink.emit(InferenceEvent::text_delta(run.clone(), 2, "second"))
        .expect("delta 2");

    assert_eq!(
        texts(&seen.borrow()),
        vec!["first second".to_owned()],
        "an elapsed interval must forward while the turn is still open"
    );
}

/// A provider that outruns the caller's flush timer still paints.
#[tokio::test(start_paused = true)]
async fn forwards_early_when_the_byte_cap_is_reached() {
    let run = run_id();
    let (seen, on_event) = recorder();
    let config = CoalesceConfig {
        min_coalesce: Duration::from_secs(5),
        max_coalesce_bytes: 8,
    };
    let mut sink = InferencePresentationSink::new(run.clone(), config, on_event);

    sink.emit(InferenceEvent::text_delta(run.clone(), 1, "1234"))
        .expect("delta 1");
    assert!(seen.borrow().is_empty(), "below the cap");

    sink.emit(InferenceEvent::text_delta(run.clone(), 2, "5678"))
        .expect("delta 2");
    assert_eq!(
        texts(&seen.borrow()),
        vec!["12345678".to_owned()],
        "reaching the byte cap forwards without waiting for the interval"
    );
}

/// A provider that goes quiet mid-answer has still produced text the reader
/// should see. The sink cannot observe elapsed time on its own, so the caller's
/// timer drives this.
#[tokio::test(start_paused = true)]
async fn flush_forwards_a_stalled_buffer() {
    let run = run_id();
    let (seen, on_event) = recorder();
    let mut sink = InferencePresentationSink::new(
        run.clone(),
        CoalesceConfig::for_delta_coalescing(),
        on_event,
    );

    sink.emit(InferenceEvent::text_delta(run.clone(), 1, "stalled"))
        .expect("delta");
    assert!(seen.borrow().is_empty());

    sink.flush();
    assert_eq!(texts(&seen.borrow()), vec!["stalled".to_owned()]);

    sink.flush();
    assert_eq!(
        texts(&seen.borrow()).len(),
        1,
        "a second flush with nothing buffered must not forward an empty delta"
    );
}

/// The frontend admits a delta whose sequence exceeds the highest it has seen,
/// so consecutive forwards have to climb.
#[tokio::test(start_paused = true)]
async fn sequences_increase_across_consecutive_forwards() {
    let run = run_id();
    let (seen, on_event) = recorder();
    let mut sink = InferencePresentationSink::new(
        run.clone(),
        CoalesceConfig::for_delta_coalescing(),
        on_event,
    );

    sink.emit(InferenceEvent::text_delta(run.clone(), 1, "a"))
        .expect("delta 1");
    sink.emit(InferenceEvent::text_delta(run.clone(), 2, "b"))
        .expect("delta 2");
    sink.flush();
    sink.emit(InferenceEvent::text_delta(run.clone(), 3, "c"))
        .expect("delta 3");
    sink.emit(InferenceEvent::text_delta(run.clone(), 4, "d"))
        .expect("delta 4");
    sink.flush();

    let sequences: Vec<u64> = seen
        .borrow()
        .iter()
        .filter_map(|event| match event {
            InferenceEvent::TextDelta { sequence, .. } => Some(*sequence),
            _ => None,
        })
        .collect();
    assert_eq!(sequences, vec![1, 3]);
    assert!(
        sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "each forward must carry a higher sequence than the one before it"
    );
}

/// Buffered prose was produced before the event that follows it, so forwarding
/// it afterwards would reorder the answer around a terminal event.
#[tokio::test(start_paused = true)]
async fn flushes_text_before_non_delta_events() {
    let run = run_id();
    let (seen, on_event) = recorder();
    let mut sink = InferencePresentationSink::new(
        run.clone(),
        CoalesceConfig::for_delta_coalescing(),
        on_event,
    );

    sink.emit(InferenceEvent::text_delta(run.clone(), 1, "a"))
        .expect("delta 1");
    sink.emit(InferenceEvent::text_delta(run.clone(), 2, "b"))
        .expect("delta 2");
    sink.emit(InferenceEvent::completed(run.clone(), FinishReason::Stop))
        .expect("completed");

    // Asserted against what was FORWARDED, not against what `finish` collected:
    // the forwarded order is the order the reader sees, and it is the one a
    // buffer that only drains at the end would get wrong.
    let forwarded = seen.borrow();
    let delta_index = forwarded
        .iter()
        .position(|event| matches!(event, InferenceEvent::TextDelta { .. }))
        .expect("text delta");
    let completed_index = forwarded
        .iter()
        .position(|event| matches!(event, InferenceEvent::Completed { .. }))
        .expect("completed");
    assert!(delta_index < completed_index);
    match &forwarded[delta_index] {
        InferenceEvent::TextDelta { text, .. } => assert_eq!(text, "ab"),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn started_event_passes_through_without_coalesce() {
    let run = run_id();
    let (seen, on_event) = recorder();
    let mut sink = InferencePresentationSink::new(
        run.clone(),
        CoalesceConfig::for_delta_coalescing(),
        on_event,
    );

    sink.emit(InferenceEvent::started(
        run.clone(),
        "req-1",
        ProviderId::OpenAi,
        ModelId::from("gpt"),
    ))
    .expect("started");

    assert!(matches!(
        seen.borrow()[0],
        InferenceEvent::Started {
            version: CONTRACT_VERSION,
            ..
        }
    ));
}

/// The zero-interval config is what lets a test assert on emitted events rather
/// than on a clock, so it must forward one delta per emit with no extra branch.
#[tokio::test(start_paused = true)]
async fn the_test_config_forwards_every_delta() {
    let run = run_id();
    let (seen, on_event) = recorder();
    let mut sink =
        InferencePresentationSink::new(run.clone(), CoalesceConfig::for_tests(), on_event);

    sink.emit(InferenceEvent::text_delta(run.clone(), 1, "a"))
        .expect("delta 1");
    sink.emit(InferenceEvent::text_delta(run.clone(), 2, "b"))
        .expect("delta 2");

    assert_eq!(texts(&seen.borrow()), vec!["a".to_owned(), "b".to_owned()]);
}

#[tokio::test(start_paused = true)]
async fn rejects_oversized_text_delta() {
    let run = run_id();
    let (_seen, on_event) = recorder();
    let mut sink =
        InferencePresentationSink::new(run.clone(), CoalesceConfig::for_tests(), on_event);

    // Compaction transport lines have a larger independent limit. Text and
    // reasoning still use the bounded presentation contract.
    let huge = "x".repeat(512 * 1024);
    let error = sink
        .emit(InferenceEvent::text_delta(run.clone(), 1, huge))
        .expect_err("oversized");
    assert_eq!(error, StreamError::OversizedPayload);

    // A large but presentation-legal chunk remains accepted.
    let large = "x".repeat(200 * 1024);
    sink.emit(InferenceEvent::text_delta(run, 2, large))
        .expect("a transport-legal chunk must not fail the run");
}

/// The caller reads its timer period from the sink rather than holding a second
/// copy that can drift from it.
#[test]
fn the_sink_reports_the_cadence_its_caller_should_flush_at() {
    let (_seen, on_event) = recorder();
    let sink =
        InferencePresentationSink::new(run_id(), CoalesceConfig::for_delta_coalescing(), on_event);
    assert_eq!(sink.coalesce_interval(), Duration::from_millis(16));
}

/// The budget, which a regression in EITHER direction has to fail.
///
/// A sink that forwards every delta is the IPC cost coalescing exists to
/// remove. A sink that forwards once at the end is the defect it exists to fix,
/// and it is what shipped: `min_coalesce` was a `Duration` no clock ever read.
#[tokio::test(start_paused = true)]
async fn forwards_more_than_once_and_fewer_times_than_it_received() {
    const TOKENS: u64 = 200;
    /// A brisk provider: 250 tokens per second against a 16 ms window.
    const TOKEN_GAP: Duration = Duration::from_millis(4);

    let run = run_id();
    let (seen, on_event) = recorder();
    let mut sink = InferencePresentationSink::new(
        run.clone(),
        CoalesceConfig::for_delta_coalescing(),
        on_event,
    );

    for sequence in 1..=TOKENS {
        sink.emit(InferenceEvent::text_delta(run.clone(), sequence, "tok "))
            .expect("delta");
        tokio::time::advance(TOKEN_GAP).await;
    }
    sink.flush();

    let forwards = texts(&seen.borrow()).len();
    assert!(
        forwards > 1,
        "one forward for a whole stream is not streaming: {forwards}"
    );
    assert!(
        forwards < TOKENS as usize,
        "a forward per token defeats coalescing entirely: {forwards}"
    );
    // Four tokens per 16 ms window puts the cadence near 40. The range is wide
    // enough that the off-by-one of checking the bound after the append does not
    // matter, and narrow enough that losing either bound fails here.
    assert!(
        (25..=70).contains(&forwards),
        "unexpected forward cadence: {forwards}"
    );
}

/// The cost this coalescing removes.
///
/// Reasoning arrives at the same token cadence as answer text, and every
/// fragment used to be one JSON serialization and one IPC hop to the webview —
/// which became the run's dominant presentation cost the moment reasoning was
/// requested from every shipped vendor. It is also content the reader may have
/// turned off, so the cost bought nothing.
#[tokio::test(start_paused = true)]
async fn coalesces_reasoning_on_the_same_bounds_as_text() {
    let run = run_id();
    let (seen, on_event) = recorder();
    let mut sink = InferencePresentationSink::new(
        run.clone(),
        CoalesceConfig::for_delta_coalescing(),
        on_event,
    );

    sink.emit_reasoning("I should ", 1).expect("fragment 1");
    sink.emit_reasoning("check the ", 2).expect("fragment 2");
    assert!(
        seen.borrow().is_empty(),
        "neither bound was met, so nothing should have been forwarded yet"
    );

    tokio::time::advance(Duration::from_millis(20)).await;
    sink.emit_reasoning("columns.", 3).expect("fragment 3");

    assert_eq!(
        reasonings(&seen.borrow()),
        vec!["I should check the columns.".to_owned()],
        "an elapsed interval must forward while the turn is still open"
    );
}

/// Reasoning and answer text land in two different places on two independent
/// sequence ledgers (`chatReduce`), so their order relative to each other is not
/// observable — but each must be whole, and neither may swallow the other.
#[tokio::test(start_paused = true)]
async fn text_and_reasoning_buffer_independently() {
    let run = run_id();
    let (seen, on_event) = recorder();
    let mut sink = InferencePresentationSink::new(
        run.clone(),
        CoalesceConfig {
            min_coalesce: Duration::from_secs(5),
            max_coalesce_bytes: 4 * 1024,
        },
        on_event,
    );

    sink.emit_reasoning("think ", 1).expect("reasoning 1");
    sink.emit(InferenceEvent::text_delta(run.clone(), 2, "answer "))
        .expect("text 1");
    sink.emit_reasoning("more", 3).expect("reasoning 2");
    sink.emit(InferenceEvent::text_delta(run.clone(), 4, "text"))
        .expect("text 2");
    assert!(seen.borrow().is_empty(), "neither bound was met");

    sink.flush();
    assert_eq!(texts(&seen.borrow()), vec!["answer text".to_owned()]);
    assert_eq!(reasonings(&seen.borrow()), vec!["think more".to_owned()]);
}

/// Buffered reasoning was produced before the event that follows it, so
/// forwarding it afterwards would file it against the wrong tool row — the
/// frontend positions a fragment by how many rows the run had drawn when it
/// arrived.
#[tokio::test(start_paused = true)]
async fn flushes_reasoning_before_non_delta_events() {
    let run = run_id();
    let (seen, on_event) = recorder();
    let mut sink = InferencePresentationSink::new(
        run.clone(),
        CoalesceConfig {
            min_coalesce: Duration::from_secs(5),
            max_coalesce_bytes: 4 * 1024,
        },
        on_event,
    );

    sink.emit_reasoning("weighing", 1).expect("reasoning");
    sink.emit(InferenceEvent::completed(run.clone(), FinishReason::Stop))
        .expect("completed");

    let forwarded = seen.borrow();
    let reasoning_index = forwarded
        .iter()
        .position(|event| matches!(event, InferenceEvent::ReasoningSummary { .. }))
        .expect("reasoning forwarded");
    let completed_index = forwarded
        .iter()
        .position(|event| matches!(event, InferenceEvent::Completed { .. }))
        .expect("completed");
    assert!(reasoning_index < completed_index);
}

/// The contract caps one `ReasoningSummary` at 4 KiB and its constructor is the
/// only way in, so the buffer has to stay under it. The previous code's answer to
/// a refusal was `.ok()` — the fragment vanished with nothing said.
#[tokio::test(start_paused = true)]
async fn never_assembles_more_reasoning_than_one_event_may_carry() {
    let run = run_id();
    let (seen, on_event) = recorder();
    let mut sink = InferencePresentationSink::new(
        run.clone(),
        CoalesceConfig {
            // A byte cap far past the contract's, so only the contract bound can
            // be what stops the buffer growing.
            min_coalesce: Duration::from_secs(5),
            max_coalesce_bytes: 1024 * 1024,
        },
        on_event,
    );

    // 12 KiB of reasoning in 1 KiB fragments: three events at most 4 KiB each,
    // and every byte accounted for.
    for sequence in 1..=12u64 {
        sink.emit_reasoning(&"x".repeat(1024), sequence)
            .expect("fragment");
    }
    sink.flush();

    let forwarded = reasonings(&seen.borrow());
    assert!(
        forwarded.len() > 1,
        "one 12 KiB event cannot be constructed"
    );
    for chunk in &forwarded {
        assert!(
            chunk.len() <= 4 * 1024,
            "an event carried {} bytes, past the contract bound",
            chunk.len()
        );
    }
    assert_eq!(
        forwarded.iter().map(String::len).sum::<usize>(),
        12 * 1024,
        "no fragment may be dropped on the way out"
    );
}

/// The same budget as the text one, for the stream that now shares its bounds.
#[tokio::test(start_paused = true)]
async fn forwards_reasoning_more_than_once_and_fewer_times_than_it_received() {
    const FRAGMENTS: u64 = 200;
    const TOKEN_GAP: Duration = Duration::from_millis(4);

    let run = run_id();
    let (seen, on_event) = recorder();
    let mut sink = InferencePresentationSink::new(
        run.clone(),
        CoalesceConfig::for_delta_coalescing(),
        on_event,
    );

    for sequence in 1..=FRAGMENTS {
        sink.emit_reasoning("tok ", sequence).expect("fragment");
        tokio::time::advance(TOKEN_GAP).await;
    }
    sink.flush();

    let forwards = reasonings(&seen.borrow()).len();
    assert!(
        forwards > 1,
        "reasoning must still arrive progressively: {forwards}"
    );
    assert!(
        forwards < FRAGMENTS as usize,
        "a forward per fragment is the cost this removes: {forwards}"
    );
}
