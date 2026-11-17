/**
 * When the turn now in progress began, for the elapsed time the run block shows.
 *
 * The waiting pill and the working card are two components drawn in turn for
 * one response, so a clock owned by either would restart at the swap. This
 * lives beside the chat store instead: `chatStore` calls `syncTurnClock` on
 * every change, starting the clock the moment a turn is claimed or a run is
 * bound and clearing it when nothing is working.
 *
 * The store shows one conversation at a time, so opening another chat looks
 * like "nothing is working". A run's start is therefore remembered by run id
 * until that run is seen to end, and coming back to a chat whose run is still
 * going picks its time up where it was instead of from zero.
 */
let startedAt: number | null = null;
/** When the turn was claimed, while it still has no run id. */
let waitingSince: number | null = null;
const runStarts = new Map<string, number>();
const MAX_REMEMBERED_RUNS = 32;

/**
 * `runId` is the run bound to the view, if any. A view with a run id that is
 * not working is that run's end; a view with none (a different chat opened) is
 * not.
 */
export function syncTurnClock(working: boolean, runId: string | null = null): void {
  if (!working) {
    if (runId !== null) runStarts.delete(runId);
    waitingSince = null;
    startedAt = null;
    return;
  }
  if (runId === null) {
    waitingSince ??= Date.now();
    startedAt = waitingSince;
    return;
  }
  const known = runStarts.get(runId);
  if (known !== undefined) {
    startedAt = known;
    return;
  }
  // First sight of this run. The wait before it had a run id is part of it.
  const start = waitingSince ?? Date.now();
  waitingSince = null;
  startedAt = start;
  runStarts.set(runId, start);
  if (runStarts.size > MAX_REMEMBERED_RUNS) {
    const oldest = runStarts.keys().next().value;
    if (oldest !== undefined) runStarts.delete(oldest);
  }
}

/** Forgets every remembered start, as signing out must. */
export function resetTurnClock(): void {
  runStarts.clear();
  waitingSince = null;
  startedAt = null;
}

export function turnStartedAt(): number | null {
  return startedAt;
}

/** "8.0s", "2m 8.0s", "1h 2m 8s": tenths while it is short enough to watch. */
export function formatTurnElapsed(milliseconds: number): string {
  const tenths = Math.max(0, Math.floor(milliseconds / 100));
  const totalSeconds = Math.floor(tenths / 10);
  const seconds = totalSeconds % 60;
  const minutes = Math.floor(totalSeconds / 60) % 60;
  const hours = Math.floor(totalSeconds / 3600);
  if (hours > 0) return `${hours}h ${minutes}m ${seconds}s`;
  const secondsLabel = `${seconds}.${tenths % 10}s`;
  return minutes > 0 ? `${minutes}m ${secondsLabel}` : secondsLabel;
}
