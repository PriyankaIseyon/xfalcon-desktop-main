import type { ToolInvocationEvent } from "../../lib/generated/contracts";
import type { ChatMessageRow, LiveSegment } from "./chatStore";

/**
 * The two questions both halves of the transcript have to answer the same way.
 *
 * They live here rather than in either component because each has two readers —
 * the durable transcript and the live timeline, or the live timeline and the
 * thinking indicator — and a predicate with two readers and two definitions is
 * how this surface came to show an empty card in place of an indicator, and two
 * response boundaries for one response.
 */

/**
 * Whether the response in progress has already drawn its boundary.
 *
 * The ribbon marks the handover from the user's turn to the final assistant
 * answer. Tool-bearing round narration is not a visible prose row, so it cannot
 * open the response before that answer (spec: unify-response-tool-section).
 *
 * `MessageList.buildDisplayItems` maintains the same fact incrementally as it
 * walks, because asking this per row would make the build quadratic. Both ranges
 * it builds start at a user message or at index 0, so both start with no
 * response open; `LiveRunTimeline.test.tsx` holds the two together across every
 * mix of durable turns rather than trusting that they agree.
 */
export function responseAlreadyOpen(messages: readonly ChatMessageRow[]): boolean {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role === "user") return false;
    if (message.role === "assistant" && message.toolCalls.length === 0 && message.text.trim().length > 0) return true;
  }
  return false;
}

/**
 * Whether the live timeline has anything to draw.
 *
 * ONE predicate, so the thinking indicator reads the same answer the timeline
 * acts on. Two conditions that agree today and are edited separately tomorrow is
 * how a run came to show an auto-expanded bordered card with an empty body in
 * place of its thinking indicator (spec: chat-run-activity, "A live surface never
 * draws an empty card").
 *
 * `showReasoning` is a parameter rather than an assumption, so turning the
 * preference off leaves the surface coherent instead of leaving an empty box.
 */
export function liveRunHasContent(
  {
    activity,
    reasoning,
    liveText,
  }: {
    activity: readonly ToolInvocationEvent[];
    roundProse: readonly LiveSegment[];
    reasoning: readonly LiveSegment[];
    liveText: string;
  },
  showReasoning: boolean,
): boolean {
  if (activity.length > 0) return true;
  if (liveText.length > 0) return true;
  return showReasoning && reasoning.length > 0;
}

/**
 * Whether the tail timeline still owns the run's content.
 *
 * Reasoning alone belongs there only while the run is active. Once the durable
 * answer arrives, the transcript owns that reasoning and places it before the
 * answer. Prose and tool activity remain tail-owned across their brief settle
 * gap because their durable replacements may not have arrived yet.
 */
export function liveTimelineOwnsRun(
  {
    activity,
    reasoning,
    liveText,
    runActive,
  }: {
    activity: readonly ToolInvocationEvent[];
    roundProse: readonly LiveSegment[];
    reasoning: readonly LiveSegment[];
    liveText: string;
    runActive: boolean;
  },
  showReasoning: boolean,
): boolean {
  // `roundProse` is retained for lifecycle/context accounting but is
  // intentionally not rendered. It therefore cannot claim the visual slot:
  // doing so hid the Thinking pill behind an empty LiveRunTimeline between
  // model rounds, then made the pill reappear on the next visible event.
  if (activity.length > 0 || liveText.length > 0) return true;
  return runActive && showReasoning && reasoning.length > 0;
}
