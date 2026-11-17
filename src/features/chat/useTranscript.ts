import { useChatSelector } from "./chatStore";
import { isBoundRunPending } from "./runState";

/**
 * The state that changes while a run streams.
 *
 * Split out of `useChat` so the shell above the transcript is not subscribed to
 * it. `useChat` holds the actions and the fields that change once per
 * conversation or once per run; everything a token can move lives here, and
 * `ChatPane` is the only component that reads it
 * (spec: chat-streaming-performance).
 *
 * Every value comes through its own `useChatSelector`, so a component wakes for
 * the fields it reads and no others. Each selector returns a store-owned
 * reference or a primitive, which is the contract `useChatSelector` documents.
 */
export function useTranscript() {
  const messages = useChatSelector((snapshot) => snapshot.messages);
  const conversationId = useChatSelector((snapshot) => snapshot.conversationId);
  const reservedConversationId = useChatSelector((snapshot) => snapshot.reservedConversationId);
  const availability = useChatSelector((snapshot) => snapshot.availability);
  const conversationSyncing = useChatSelector((snapshot) => snapshot.openingConversationId !== null);
  const error = useChatSelector((snapshot) => snapshot.error);
  const recoveryNotice = useChatSelector((snapshot) => snapshot.recoveryNotice);
  const activeRunId = useChatSelector((snapshot) => snapshot.activeRunId);
  const runState = useChatSelector((snapshot) => snapshot.runState);
  const streamingText = useChatSelector((snapshot) => snapshot.streamingText);
  const settlingText = useChatSelector((snapshot) => snapshot.settlingText);
  const activity = useChatSelector((snapshot) => snapshot.activity);
  const roundProse = useChatSelector((snapshot) => snapshot.roundProse);
  const reasoning = useChatSelector((snapshot) => snapshot.reasoning);
  const reasoningTrimmed = useChatSelector((snapshot) => snapshot.reasoningTrimmed);
  const revising = useChatSelector((snapshot) => snapshot.revising);
  const awaitingRun = useChatSelector((snapshot) => snapshot.awaitingRun);
  const conversationContentPending = useChatSelector((snapshot) => snapshot.conversationContentPending);

  // The run's answer while it is still the run's to show: streaming, then held
  // across the gap between the terminal inference event and the durable row.
  // Both halves retire on the SAME signal as the activity feed, so the live
  // surfaces and the durable row can never both carry it — that overlap is how
  // the same answer reached the screen twice (spec: chat-run-activity).
  //
  // ONE value, for ONE surface. It used to be split in two by whether the run
  // had called a tool, which sent the answer into the activity card for a
  // tool-bearing run and left it in the transcript otherwise — a decision that
  // is unknown when the first token arrives and was therefore revised while the
  // reader watched (spec: chat-run-activity, "Streamed content renders where its
  // durable form lives").
  const liveText = streamingText || settlingText;

  return {
    messages,
    conversationId,
    reservedConversationId,
    availability,
    conversationSyncing,
    error,
    recoveryNotice,
    activeRunId,
    activity,
    roundProse,
    reasoning,
    reasoningTrimmed,
    revising,
    /** A turn is on its way to the backend and has no run yet. */
    awaitingRun,
    conversationContentPending,
    /** The in-flight prose, for the live timeline that renders it as a prose
     * row. Empty once the durable row owns it. */
    liveText,
    streamRunId: activeRunId,
    pending: isBoundRunPending(activeRunId, runState),
    streaming: Boolean(streamingText),
  };
}
