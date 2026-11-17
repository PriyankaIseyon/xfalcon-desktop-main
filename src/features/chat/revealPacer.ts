/**
 * Paces how fast streamed answer text is revealed.
 *
 * Providers deliver text in uneven clumps: a fast model sends a burst, then
 * nothing for a few frames, then another burst. Showing each clump as it lands
 * reads as stutter. The pacer instead aims to show text a short, fixed time
 * after it arrives, so the reveal speed follows the arrival rate and each clump
 * becomes a brief typewriter run rather than a block.
 *
 * It is a proportional controller on the unrevealed backlog. The target speed
 * is the backlog divided by `LAG_SECONDS`, so a steady stream settles at a
 * constant lag and a burst is drained fast but visibly, in a fraction of a
 * second. The speed then eases toward its target, so it never jumps between
 * frames. Time is measured in seconds, not frames, so 60 Hz and 120 Hz displays
 * reveal at the same pace.
 *
 * The backlog still scales the speed past the typewriter ceiling, so a very
 * large burst clears within `MAX_QUEUE_SECONDS` instead of queueing behind a
 * fixed character rate (spec: chat-streaming-performance).
 */

/** How long text waits, at steady state, between arriving and being shown. */
const LAG_SECONDS = 0.2;
/** Once the provider has finished nothing more is coming, so the backlog drains faster. */
const SETTLING_LAG_SECONDS = 0.1;
/** Floor, so the tail of a burst finishes promptly instead of trailing off. */
const MIN_CHARS_PER_SECOND = 120;
/** Typewriter ceiling for ordinary backlogs: about 37 characters per 60 Hz frame. */
const SOFT_MAX_CHARS_PER_SECOND = 2200;
/** A backlog above the ceiling's reach may run faster so it still clears in this time. */
const MAX_QUEUE_SECONDS = 0.6;
const SETTLING_MAX_QUEUE_SECONDS = 0.3;
/** Time constants for the speed itself, so it ramps up quickly but eases down. */
const SPEED_UP_SECONDS = 0.09;
const SPEED_DOWN_SECONDS = 0.28;
/** A stalled or throttled frame must not teleport the reveal forward. */
const MAX_FRAME_SECONDS = 0.1;
/** The step used for the first frame, which has no previous frame to measure. */
const FIRST_FRAME_SECONDS = 1 / 60;

export type RevealPacer = {
  /** Characters revealed so far. Fractional, so slow speeds still advance. */
  shown: number;
  /** Current reveal speed in characters per second. */
  speed: number;
  /** Timestamp of the previous frame in milliseconds, or null before the first. */
  lastTime: number | null;
};

export function startPacer(shown = 0): RevealPacer {
  return { shown, speed: MIN_CHARS_PER_SECOND, lastTime: null };
}

/**
 * Advances the pacer to the frame at `now` (milliseconds) for a source `length`
 * characters long. `settling` means the provider has finished, so the remaining
 * backlog is drained quickly.
 */
export function advancePacer(pacer: RevealPacer, now: number, length: number, settling = false): RevealPacer {
  const seconds =
    pacer.lastTime === null
      ? FIRST_FRAME_SECONDS
      : Math.min(Math.max((now - pacer.lastTime) / 1000, 0), MAX_FRAME_SECONDS);
  const backlog = length - pacer.shown;
  // Caught up: there is no momentum to carry into the next burst.
  if (backlog <= 0) return { shown: length, speed: MIN_CHARS_PER_SECOND, lastTime: now };

  const ceiling = Math.max(
    SOFT_MAX_CHARS_PER_SECOND,
    backlog / (settling ? SETTLING_MAX_QUEUE_SECONDS : MAX_QUEUE_SECONDS),
  );
  const target = Math.min(
    ceiling,
    Math.max(MIN_CHARS_PER_SECOND, backlog / (settling ? SETTLING_LAG_SECONDS : LAG_SECONDS)),
  );
  const timeConstant = target > pacer.speed ? SPEED_UP_SECONDS : SPEED_DOWN_SECONDS;
  const speed = pacer.speed + (target - pacer.speed) * (1 - Math.exp(-seconds / timeConstant));
  return { shown: Math.min(length, pacer.shown + speed * seconds), speed, lastTime: now };
}

/**
 * How many leading characters of `text` to show for a pacer position.
 *
 * Never ends between the two halves of a UTF-16 surrogate pair, so an emoji does
 * not flash as a replacement character for one frame.
 */
export function revealEnd(text: string, shown: number): number {
  let end = Math.min(text.length, Math.floor(shown));
  if (end <= 0 || end >= text.length) return end;
  const finalCodeUnit = text.charCodeAt(end - 1);
  if (finalCodeUnit >= 0xd800 && finalCodeUnit <= 0xdbff) end += 1;
  return end;
}
