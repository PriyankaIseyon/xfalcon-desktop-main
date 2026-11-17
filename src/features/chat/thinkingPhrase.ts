import { useEffect, useRef, useState } from "react";
import { useChatSelector } from "./chatStore";
import { turnStartedAt } from "./runClock";

/**
 * What the run block says while it is working.
 *
 * A fixed cycle, deliberately: it reports that the app is working, never what
 * the run is doing. The run's own state is inside the block — the reasoning it
 * wrote, the tools it called — and a header that narrated the same thing renamed
 * itself two or three times inside one answer, which read as the app changing
 * its mind rather than as one response arriving. Mirrors xfalcon-web's
 * `THINKING_TEXTS`.
 *
 * One word each: the pill holds one width while they rotate, and a long phrase
 * either widened it or left a gap beside a short one. "Thinking" leads because
 * it is what a submitted turn shows before its run exists.
 */
export const THINKING_PHRASES = ["Thinking", "Working", "Reasoning", "Analyzing", "Processing"] as const;

/** The longest of the cycle, which a label reserves room for so the pill keeps
 * one width while the phrases rotate. */
export const LONGEST_THINKING_PHRASE = THINKING_PHRASES.reduce((longest, phrase) =>
  phrase.length > longest.length ? phrase : longest,
);

/**
 * What the block says while the run shortens its history to fit the model's
 * window. The one exception to the fixed cycle: it is a single step that can
 * take long enough to look like a stall, and it ends on its own.
 */
export const COMPACTING_PHRASE = "Summarizing";

/** How long one phrase holds. Long enough to read, short enough to stay alive. */
export const PHRASE_HOLD_MS = 2000;

/** Rotation is motion, so a reader who asked for none keeps the first phrase.
 * Read the same way `MessageList` reads it for scroll behaviour. */
function prefersReducedMotion(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** The phrase to show right now. One timer per mounted block; a block that
 * settles unmounts its rotation with it. */
export function useThinkingPhrase(): string {
  const [phraseIndex, setPhraseIndex] = useState(0);

  useEffect(() => {
    if (prefersReducedMotion()) return;
    const rotation = setInterval(
      () => setPhraseIndex((current) => (current + 1) % THINKING_PHRASES.length),
      PHRASE_HOLD_MS,
    );
    return () => clearInterval(rotation);
  }, []);

  const compacting = useChatSelector(
    (snapshot) => snapshot.compactingRunId !== null && snapshot.compactingRunId === snapshot.activeRunId,
  );
  return compacting ? COMPACTING_PHRASE : THINKING_PHRASES[phraseIndex];
}

/** How often the stopwatch redraws: often enough for the tenths to move. */
const STOPWATCH_TICK_MS = 100;

/**
 * Milliseconds the turn has been going, redrawn while the run block is mounted.
 *
 * Display only, like the phrase rotation above: the figure is the client's own
 * observation, nothing reads it back, and the timer is cleared on unmount. The
 * turn's start is `runClock`'s, so it keeps counting across the waiting pill
 * giving way to the working card.
 */
export function useTurnElapsedMs(): number {
  const mountedAt = useRef(Date.now());
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const tick = setInterval(() => setNow(Date.now()), STOPWATCH_TICK_MS);
    return () => clearInterval(tick);
  }, []);

  return now - (turnStartedAt() ?? mountedAt.current);
}
