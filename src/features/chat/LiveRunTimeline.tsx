import { useMemo } from "react";
import type { ToolInvocationEvent } from "../../lib/generated/contracts";
import { useShowReasoning } from "../../app/appearance";
import type { LiveSegment } from "./chatStore";
import { LiveRunActivity, type RunAside } from "./ToolActivityCard";

export type LiveRunTimelineProps = {
  /** Live tool-invocation events for this conversation, oldest first. */
  activity: readonly ToolInvocationEvent[];
  /** The run's reasoning, split where tool rows interrupted it. */
  reasoning: readonly LiveSegment[];
  /** The retention cap dropped the oldest reasoning. */
  reasoningTrimmed: boolean;
  /** The run is still working. */
  runActive: boolean;
};

/**
 * The run's thinking, in produced order, for one pill.
 *
 * Tools share that pill. Tool-round narration stays out of the transcript, so
 * one model loop cannot look like several assistant responses.
 */
function buildReasoningAsides(
  reasoning: readonly LiveSegment[],
  reasoningTrimmed: boolean,
  showReasoning: boolean,
): RunAside[] {
  if (!showReasoning) return [];
  return reasoning.map((segment, index) => ({
    kind: "reasoning" as const,
    text: segment.text,
    afterCallCount: segment.afterCallCount,
    trimmed: reasoningTrimmed && index === 0,
  }));
}

/**
 * The run in flight: one work pill that owns thinking and every tool call.
 *
 * The answer is not drawn here. It is `CurrentAnswer`, one element that the
 * transcript keeps from the first token to the saved row, so nothing about it is
 * rebuilt when the run settles. When the durable snapshot arrives this pill
 * becomes the trail above that answer, so the chrome does not change family
 * (spec: unify-response-tool-section).
 *
 * It renders OUTSIDE the virtualized window, which is what lets the window hold
 * the durable conversation alone: a streamed delta then changes no virtual row
 * count and no row key.
 */
export function LiveRunTimeline({ activity, reasoning, reasoningTrimmed, runActive }: LiveRunTimelineProps) {
  const showReasoning = useShowReasoning();
  const asides = useMemo(
    () => buildReasoningAsides(reasoning, reasoningTrimmed, showReasoning),
    [reasoning, reasoningTrimmed, showReasoning],
  );
  if (asides.length === 0 && activity.length === 0) return null;
  return (
    <div className="py-1.5">
      <LiveRunActivity asides={asides} events={activity} runActive={runActive} />
    </div>
  );
}
