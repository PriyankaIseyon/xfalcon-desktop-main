import type { RunState } from "../../lib/generated/contracts";

/**
 * Run states the UI must still treat as working.
 *
 * Deliberately narrower than the Rust `agent::recovery::is_in_flight_run_state`,
 * which also counts `interrupted` because such a run keeps its journal and
 * stays resumable: once a run is interrupted the transcript shows the recovery
 * notice and the composer has to come back, so the UI calls it settled.
 * `awaiting_approval` IS pending — the run is parked on a tool prompt, not
 * finished, and the `Running -> AwaitingApproval -> Running` transition is
 * emitted to this view like any other.
 */
const PENDING_RUN_STATES = new Set<string>([
  "draft",
  "queued",
  "running",
  "awaiting_approval",
  "recovering",
] satisfies RunState[]);

/** Whether the run bound to the view is still working — the single predicate
 * behind the composer's send/stop affordance, the live activity card's
 * open-while-running choreography, and the retirement of live run state. */
export function isRunPending(state: string | null): boolean {
  return state !== null && PENDING_RUN_STATES.has(state);
}

/** Whether the run bound to this transcript still owns the turn.
 *
 * A projection can identify the active run before its lifecycle event arrives,
 * so `null` means "not classified yet" when a run id exists. Treating the same
 * pair differently in the store and the UI made the pill, Stop control, and
 * focus handoff disagree during that event gap. */
export function isBoundRunPending(runId: string | null, state: string | null): runId is string {
  return runId !== null && (state === null || isRunPending(state));
}
