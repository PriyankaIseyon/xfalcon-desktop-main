import { RotatingLabel } from "./RotatingLabel";
import { formatTurnElapsed } from "./runClock";
import { useTurnElapsedMs } from "./thinkingPhrase";

/**
 * What the run block says while it works: the rotating phrase and, beside it,
 * how long the turn has been going. One element, so the phrase and the clock
 * read as one status rather than a label with a counter attached.
 */
export function ThinkingStatus({ phrase }: { phrase: string }) {
  const elapsed = useTurnElapsedMs();
  return (
    <span className="thinking-status">
      <RotatingLabel phrase={phrase} />
      <time className="thinking-status__timer">{formatTurnElapsed(elapsed)}</time>
    </span>
  );
}
