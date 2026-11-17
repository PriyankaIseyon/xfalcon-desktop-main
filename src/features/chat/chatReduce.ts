import type { InferenceEvent, RunEvent, ToolInvocationEvent } from "../../lib/generated/contracts";
import type { ChatStoreSnapshot, LiveSegment } from "./chatStore";
import { recoveryNoticeForRunState } from "./recoveryPolicy";

/**
 * Per-run stream watermarks. A known run may keep streaming after its
 * conversation leaves the screen, so one global pair would let interleaved
 * conversations overwrite each other's de-duplication state.
 *
 * The store bounds retained live runs. This defensive cap also bounds direct
 * reducer use in tests and sheds the oldest completed-or-abandoned watermark.
 */
const MAX_SEQUENCE_RUNS = 32;
const inferenceSequences = new Map<string, number>();
const reasoningSequences = new Map<string, number>();
const terminalInferenceRuns = new Set<string>();

export type InferenceSequenceSnapshot = {
  text: number | null;
  reasoning: number | null;
  terminal: boolean;
};

function recordSequence(ledger: Map<string, number>, runId: string, sequence: number): boolean {
  const previous = ledger.get(runId);
  if (previous !== undefined && sequence <= previous) return false;
  ledger.delete(runId);
  ledger.set(runId, sequence);
  if (ledger.size > MAX_SEQUENCE_RUNS) {
    const oldest = ledger.keys().next().value;
    if (oldest !== undefined) ledger.delete(oldest);
  }
  return true;
}

export function forgetInferenceSequenceState(runId: string): void {
  inferenceSequences.delete(runId);
  reasoningSequences.delete(runId);
  terminalInferenceRuns.delete(runId);
}

export function captureInferenceSequenceState(runId: string): InferenceSequenceSnapshot {
  return {
    text: inferenceSequences.get(runId) ?? null,
    reasoning: reasoningSequences.get(runId) ?? null,
    terminal: terminalInferenceRuns.has(runId),
  };
}

export function restoreInferenceSequenceState(runId: string, snapshot: InferenceSequenceSnapshot): void {
  forgetInferenceSequenceState(runId);
  if (snapshot.text !== null) recordSequence(inferenceSequences, runId, snapshot.text);
  if (snapshot.reasoning !== null) recordSequence(reasoningSequences, runId, snapshot.reasoning);
  if (snapshot.terminal) recordTerminal(runId);
}

export function resetInferenceSequenceState(): void {
  inferenceSequences.clear();
  reasoningSequences.clear();
  terminalInferenceRuns.clear();
  rowCountMemo = null;
}

function recordTerminal(runId: string): void {
  terminalInferenceRuns.delete(runId);
  terminalInferenceRuns.add(runId);
  if (terminalInferenceRuns.size > MAX_SEQUENCE_RUNS) {
    const oldest = terminalInferenceRuns.values().next().value;
    if (oldest !== undefined) terminalInferenceRuns.delete(oldest);
  }
}

/**
 * Cap on the reasoning retained for one run.
 *
 * Bounds retained characters rather than exact UTF-8 bytes: reasoning arrives
 * one fragment at a time, and re-encoding the whole string to measure it per
 * fragment would be quadratic on the render path. 32 Ki characters is at most
 * 64 KiB of UTF-16, which is what a bound is for.
 */
const MAX_REASONING_CHARS = 32 * 1024;

/**
 * How many tool rows the run has drawn so far — the axis every live segment is
 * placed on (spec: chat-run-activity).
 *
 * Counts distinct invocations rather than events, because that is what becomes
 * a row: one invocation emits `requested`, then `completed`, and both are in
 * `activity`. Memoized on the array's identity, which changes only when a tool
 * event arrives — reasoning fragments arrive at token cadence and must not each
 * pay for a rescan.
 */
let rowCountMemo: { activity: readonly ToolInvocationEvent[]; count: number } | null = null;

export function toolRowCount(activity: readonly ToolInvocationEvent[]): number {
  if (rowCountMemo?.activity === activity) return rowCountMemo.count;
  const seen = new Set<string>();
  for (const event of activity) {
    seen.add(event.invocation_id);
  }
  rowCountMemo = { activity, count: seen.size };
  return seen.size;
}

/**
 * Appends a fragment to the run's reasoning, in place among the tool rows.
 *
 * A fragment extends the last segment while no tool row has appeared since it
 * began; once one has, it opens a new segment, which is what puts a later
 * round's thinking BELOW the tools it reacted to instead of back at the top of
 * one growing block.
 *
 * The cap drops from the OLDEST end — the newest text is what a reader
 * following a live run is watching — shedding whole spent segments first and
 * then cutting into the oldest survivor.
 */
function appendReasoning(
  segments: readonly LiveSegment[],
  fragment: string,
  afterCallCount: number,
): { segments: LiveSegment[]; trimmed: boolean } {
  const next = segments.slice();
  const last = next[next.length - 1];
  if (last !== undefined && last.afterCallCount === afterCallCount) {
    next[next.length - 1] = { ...last, text: last.text + fragment };
  } else {
    next.push({ text: fragment, afterCallCount });
  }

  let total = 0;
  for (const segment of next) {
    total += segment.text.length;
  }
  if (total <= MAX_REASONING_CHARS) {
    return { segments: next, trimmed: false };
  }

  let excess = total - MAX_REASONING_CHARS;
  let dropped = 0;
  while (dropped < next.length && excess >= next[dropped].text.length) {
    excess -= next[dropped].text.length;
    dropped += 1;
  }
  const kept = next.slice(dropped);
  if (excess > 0 && kept.length > 0) {
    const oldest = kept[0];
    // Never begin the kept text on the tail half of a surrogate pair.
    const code = oldest.text.charCodeAt(excess);
    const cut = code >= 0xdc00 && code <= 0xdfff ? excess + 1 : excess;
    kept[0] = { ...oldest, text: oldest.text.slice(cut) };
  }
  return { segments: kept, trimmed: true };
}

export function reduceInferenceEvent(state: ChatStoreSnapshot, event: InferenceEvent): Partial<ChatStoreSnapshot> {
  // The inference channel is a broadcast: a run streaming for another
  // conversation (or a stale, superseded run) emits here too. Only the run
  // bound to the view — adopted from the submit result or the
  // conversation-scoped `run_state_changed` projection — may touch state.
  if (state.activeRunId === null || event.run_id !== state.activeRunId) {
    return {};
  }
  if (terminalInferenceRuns.has(event.run_id)) return {};
  if (event.type === "text_delta") {
    if (!recordSequence(inferenceSequences, event.run_id, event.sequence)) return {};
    return { streamingText: state.streamingText + event.text };
  }
  if (event.type === "reasoning_summary") {
    if (!recordSequence(reasoningSequences, event.run_id, event.sequence)) return {};
    const appended = appendReasoning(state.reasoning, event.summary, toolRowCount(state.activity));
    return {
      reasoning: appended.segments,
      reasoningTrimmed: state.reasoningTrimmed || appended.trimmed,
    };
  }
  if (event.type === "context_compaction") {
    return { compactingRunId: event.active ? event.run_id : null };
  }
  if (event.type === "completed") {
    recordTerminal(event.run_id);
    // The answer stops streaming here but its durable row has not landed —
    // `finish_run` reconciles after this event, with retries. Hand the text
    // to `settlingText` so the transcript does not blank across that gap;
    // `applyConversation` retires it when the durable row arrives.
    //
    // `roundProse` is NOT cleared here, for exactly the same reason. It is
    // on-screen prose whose durable replacement has not arrived either, and it
    // retires on the one signal every live field of a run retires on — the
    // durable snapshot (spec: chat-run-activity, "Every live surface of a run
    // retires on one signal"). Clearing it here blinked a round's narration off
    // the screen for the length of the reconcile.
    return { streamingText: "", settlingText: state.streamingText, compactingRunId: null };
  }
  if (event.type === "failed") {
    recordTerminal(event.run_id);
    // A dead run's partial text is not an answer — it is not held across the
    // gap, and the notice takes the surface instead. The event carries the
    // user-facing reason; dropping it leaves a sent message with no visible
    // outcome at all.
    //
    // The completed rounds' prose stays, like `activity` does: those rounds
    // happened, their durable record will carry them, and deleting the account
    // of what the run did before it died leaves the notice explaining a failure
    // with no visible history behind it.
    return { streamingText: "", settlingText: "", compactingRunId: null, error: event.message };
  }
  return {};
}

export function reduceRunEvent(state: ChatStoreSnapshot, event: RunEvent): Partial<ChatStoreSnapshot> {
  if (event.type === "state_changed") {
    // Same broadcast rule as inference: a foreign run's lifecycle must not
    // flip this view's `pending`/cancel affordances (a background run once
    // disabled the composer on a brand-new chat this way).
    if (state.activeRunId === null || event.run_id !== state.activeRunId) {
      return {};
    }
    const patch: Partial<ChatStoreSnapshot> = {
      activeRunId: event.run_id,
      runState: event.state,
    };
    patch.recoveryNotice = recoveryNoticeForRunState(event.state);
    if (event.state === "completed" || event.state === "cancelled") {
      patch.recoveryNotice = null;
      patch.error = null;
      if (event.state === "completed") {
        patch.activeRunId = null;
      }
    }
    return patch;
  }
  return {};
}
