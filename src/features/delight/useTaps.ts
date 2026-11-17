import { useCallback, useRef } from "react";

/** The longest pause between two taps that still counts as one run of them. */
export const TAP_GAP_MS = 1500;

/**
 * A tap handler that calls `onReached` on the `count`th tap in a row. A pause
 * longer than `TAP_GAP_MS` starts the run over, and reaching the count does too.
 * It draws nothing and sets no timer: it only compares clock readings.
 */
export function useTaps(count: number, onReached: () => void): () => void {
  const run = useRef({ taps: 0, last: 0 });
  const reached = useRef(onReached);
  reached.current = onReached;

  return useCallback(() => {
    const now = Date.now();
    run.current.taps = now - run.current.last > TAP_GAP_MS ? 1 : run.current.taps + 1;
    run.current.last = now;
    if (run.current.taps >= count) {
      run.current.taps = 0;
      reached.current();
    }
  }, [count]);
}
