import type { ChatMessageRow } from "./chatStore";
import { MessageRow } from "./MessageRow";
import { useSmoothStreamingText } from "./useSmoothStreamingText";

/** The answer while its run is still the run's to show: streaming, then held
 * across the gap before the saved row lands (spec: chat-run-activity). */
export type LiveAnswer = {
  text: string;
  /** The provider is still appending prose. */
  streaming: boolean;
};

/** The saved row that replaces the live answer, as the transcript built it. */
export type SavedAnswer = {
  message: ChatMessageRow;
  isLastProse: boolean;
};

export type CurrentAnswerProps = {
  live: LiveAnswer | null;
  saved: SavedAnswer | null;
  /** Position among the current run's blocks, for the transcript's own bookkeeping. */
  index: number;
  conversationId: string | null;
  showRibbon: boolean;
  /** Entrance class from the list, for a row that arrives without a live phase. */
  className?: string;
  onEdit?: (messageId: string, text: string) => void;
  onRegenerate?: (messageId: string) => void;
  revisionPending?: boolean;
};

/** Not a stored id: the `stream-` prefix is how a row says it has no saved copy yet. */
const LIVE_ANSWER_ID = "stream-answer";

/**
 * The newest run's answer: ONE element from its first token to its saved row.
 *
 * The answer used to be drawn by two different elements in two different places,
 * the live row in the tail and then the saved row in the run's blocks. Swapping
 * them rebuilt the whole markdown subtree: the text vanished and painted again,
 * so a finished answer looked as if it had been generated twice. Here the live
 * text and the saved message are only two sources for the same row. The list
 * keeps this element under one key, so React keeps the DOM, the reveal in
 * progress, and the parsed blocks across the handover. Only the props change: the
 * streaming caret goes, and the saved row's actions arrive.
 *
 * The saved text is the source once the live text retires. It is normally the
 * same string, so nothing moves. The reveal also finishes before the row takes
 * its saved identity, so an action never reads or copies half an answer.
 */
export function CurrentAnswer({
  live,
  saved,
  index,
  conversationId,
  showRibbon,
  className,
  onEdit,
  onRegenerate,
  revisionPending = false,
}: CurrentAnswerProps) {
  const source = live !== null && live.text.length > 0 ? live.text : (saved?.message.text ?? "");
  const displayed = useSmoothStreamingText(source, live?.streaming ?? false);
  // The provider can finish before the reveal does. The row keeps its streaming
  // presentation until the whole answer is on screen.
  const revealing = (live?.streaming ?? false) || displayed.length < source.length;
  const settled = saved !== null && live === null && !revealing;
  const message: ChatMessageRow = settled
    ? saved.message
    : { id: LIVE_ANSWER_ID, role: "assistant", text: displayed, sequence: BigInt(0), toolCalls: [] };

  return (
    <div aria-live={revealing ? "polite" : undefined} className={className} data-current-run-index={index}>
      <MessageRow
        conversationId={conversationId}
        isLastProse={settled ? saved.isLastProse : false}
        isLatestAssistant
        message={message}
        revisionPending={revisionPending}
        showRibbon={showRibbon}
        streaming={revealing}
        onEdit={onEdit}
        onRegenerate={onRegenerate}
      />
    </div>
  );
}
