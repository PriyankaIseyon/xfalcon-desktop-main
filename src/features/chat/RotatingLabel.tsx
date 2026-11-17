import { LONGEST_THINKING_PHRASE } from "./thinkingPhrase";

/**
 * The run block's header text. It reserves the width of the longest phrase in
 * the cycle, so the pill does not grow and shrink every two seconds as the
 * words change; a phrase longer than that (the compaction notice) widens it
 * for as long as it is shown.
 */
export function RotatingLabel({ phrase }: { phrase: string }) {
  return (
    <span className="rotating-label" data-reserve={LONGEST_THINKING_PHRASE}>
      {phrase}
    </span>
  );
}
