import { LatticeLoader } from "../../ui/LatticeLoader";
import { ToolActivityShell } from "./ToolActivityCard";
import { ThinkingStatus } from "./ThinkingStatus";
import { LATTICE } from "./latticeStyle";
import { useThinkingPhrase } from "./thinkingPhrase";

/**
 * The run block before the run has produced anything: a submitted turn whose
 * work has not started, so a sent message is never followed by dead air.
 *
 * The same box as the working block, drawn through the same shell with nothing
 * to disclose yet — not a second indicator that resembles it. It used to be a
 * translucent blurred pill with pulsing dots, which the first arriving fragment
 * replaced with a bordered card: two visual languages for two moments of one
 * response (spec: chat-run-activity, "one block per response"). Now the box
 * that is already on screen grows a chevron and a body instead.
 *
 * Pure presentation — run state lives in `chatStore`.
 */
export function ThinkingPill() {
  const phrase = useThinkingPhrase();

  return (
    <div aria-live="polite" className="py-1.5" role="status">
      {/* The phrases are one message told five ways. Announcing each rotation
          would repeat that message every two seconds, so the live region
          carries one stable sentence and the visible cycle is decorative. */}
      <span className="sr-only">Working on the request</span>
      <div aria-hidden>
        <ToolActivityShell
          bareLeading
          disclosable={false}
          label={<ThinkingStatus phrase={phrase} />}
          leading={<LatticeLoader className="tool-activity-shell__spinner" {...LATTICE} />}
        />
      </div>
    </div>
  );
}
