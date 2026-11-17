import { useMemo } from "react";
import type { ComposerProps } from "./Composer";
import { ComposerDock } from "./ComposerDock";
import { EmptyConversation } from "./EmptyConversation";
import { LiveRunTimeline } from "./LiveRunTimeline";
import { FollowUpChips } from "./FollowUpChips";
import { MessageFeedbackProvider } from "./MessageFeedback";
import { MessageList } from "./MessageList";
import { RunNotice } from "./RunNotice";
import { ThinkingPill } from "./ThinkingPill";
import type { ChatMessageRow, LiveSegment } from "./chatStore";
import { liveTimelineOwnsRun } from "./runSurface";
import type { ToolInvocationEvent } from "../../lib/generated/contracts";
import { useShowFollowUps, useShowReasoning } from "../../app/appearance";
import { Alert } from "../../ui/Alert";
import { ToolApprovalPrompt } from "../approvals/ToolApprovalPrompt";

export type ChatSurfaceProps = Pick<
  ComposerProps,
  | "disabled"
  | "onSubmit"
  | "activeProjectName"
  | "onClearProject"
  | "streaming"
  | "submitting"
  | "onStop"
  | "stopping"
  | "projectFiles"
  | "mcpServers"
  | "skills"
  | "onSlashCommand"
  | "onAttach"
  | "onPasteFiles"
  | "attachmentPending"
  | "onCancelUpload"
  | "onRetryUpload"
> & {
  mcpServerId?: string | null;
  mcpSelectionLoading?: boolean;
  mcpSelectionError?: string | null;
  mcpSelectionDisabled?: boolean;
  onMcpServerSelect?: (serverId: string | null) => void;
  error?: string | null;
  messages?: ChatMessageRow[];
  /** The open conversation, carried to the transcript only so an HTML document
   * in an answer can name its own source when saved to a project. */
  conversationId?: string | null;
  /** The conversation holding the new-chat screen's attachments, if any. The
   * composer shows its files and its model until the first send opens it. */
  reservedConversationId?: string | null;
  availabilityStale?: boolean;
  availabilitySyncing?: boolean;
  /** The locally bound project exists, but its trusted folder cannot be
   * reopened on this device. The transcript remains readable; local tools
   * fail closed until the original location is available again. */
  projectUnavailable?: boolean;
  followStream?: boolean;
  /** The in-flight prose — the run's narration or its answer, whichever is
   * arriving. Rendered by the current answer row, the same element that draws
   * its durable replacement (spec: chat-run-activity). */
  liveText?: string;
  /** The provider is still appending to `liveText`. Kept separate from the
   * composer's broader pending state so settling text flushes immediately. */
  answerStreaming?: boolean;
  /** Identifies that in-flight row. */
  streamRunId?: string | null;
  recoveryNotice?: string | null;
  /** A retry is waiting for the secure runtime to accept or reject it. */
  retryingRun?: boolean;
  onRetryRun?: () => void;
  /** Clears BOTH the error and the recovery notice — they are two reports of
   * one death and the surface shows them as one card. */
  onDismissNotice?: () => void;
  viewerName?: string | null;
  onOpenSettings?: () => void;
  /** A run is in flight and nothing it produced is on screen yet. */
  thinking?: boolean;
  /** Live tool-invocation events for this conversation (oldest first). */
  activity?: ToolInvocationEvent[];
  /** The conversation's run is in flight — drives the activity card's
   * open-while-working / close-on-finish contract (spec:
   * chat-run-activity). */
  runActive?: boolean;
  /** Prose of rounds this run already completed, frozen at each round
   * boundary. */
  roundProse?: LiveSegment[];
  /** The run's streamed reasoning, split where tool rows interrupted it.
   * Empty for a run that produced none (spec: chat-reasoning-panel). */
  reasoning?: LiveSegment[];
  /** The reasoning retention cap dropped the oldest text. */
  reasoningTrimmed?: boolean;
  onEdit?: (messageId: string, text: string) => void;
  onRegenerate?: (messageId: string) => void;
  revisionPending?: boolean;
};

/** Stable empty lists. A `= []` default builds a fresh array on every render, so
 * a caller that omits one of these props would re-key every memo downstream that
 * reads it — once per streamed token. */
const NO_SEGMENTS: LiveSegment[] = [];
const NO_EVENTS: ToolInvocationEvent[] = [];

export function ChatSurface({
  disabled,
  onSubmit,
  activeProjectName,
  onClearProject,
  streaming = false,
  submitting = false,
  onStop,
  stopping,
  error,
  messages = [],
  conversationId = null,
  reservedConversationId = null,
  availabilityStale = false,
  availabilitySyncing = false,
  projectUnavailable = false,
  followStream = false,
  liveText = "",
  answerStreaming = false,
  streamRunId = null,
  recoveryNotice,
  retryingRun = false,
  onRetryRun,
  onDismissNotice,
  viewerName,
  onOpenSettings,
  thinking = false,
  activity = NO_EVENTS,
  runActive = false,
  roundProse = NO_SEGMENTS,
  reasoning = NO_SEGMENTS,
  reasoningTrimmed = false,
  onEdit,
  onRegenerate,
  revisionPending = false,
  projectFiles,
  mcpServers,
  skills,
  onSlashCommand,
  onAttach,
  onPasteFiles,
  attachmentPending,
  onCancelUpload,
  onRetryUpload,
  mcpServerId,
  mcpSelectionLoading,
  mcpSelectionError,
  mcpSelectionDisabled,
  onMcpServerSelect,
}: ChatSurfaceProps) {
  const showReasoning = useShowReasoning();
  const showFollowUps = useShowFollowUps();
  const canSuggest = !disabled && onSubmit !== undefined;
  const lastMessage = messages.at(-1);
  const followUpAnswer =
    showFollowUps &&
    canSuggest &&
    !availabilityStale &&
    !projectUnavailable &&
    !streaming &&
    !submitting &&
    !runActive &&
    !thinking &&
    !revisionPending &&
    !liveText &&
    !error &&
    !recoveryNotice &&
    lastMessage?.role === "assistant" &&
    lastMessage.text.trim() &&
    !lastMessage.id.startsWith("stream-") &&
    !lastMessage.id.startsWith("local-")
      ? lastMessage
      : null;
  const lastUserContent = useMemo(() => {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      if (messages[index].role === "user") return messages[index].text;
    }
    return null;
  }, [messages]);

  // Exactly one surface draws the run, and reasoning goes wherever that is. The
  // live timeline holds it while the run is still on screen as a live run; once
  // that timeline retires — its tool events cleared by the durable snapshot, or
  // the run settled without ever calling anything — the transcript takes it
  // over. Handing it to both would draw the same thinking twice
  // (spec: chat-run-activity).
  const liveTimelineShown = liveTimelineOwnsRun(
    { activity, roundProse, reasoning, liveText, runActive },
    showReasoning,
  );
  // Anything the run has produced counts as content: a run that reaches the
  // screen before its optimistic user row lands must show the transcript, not
  // the welcome screen. Read from the timeline's own predicate rather than from
  // the streamed text alone, because the transcript is where the timeline
  // renders — a run whose first round finished before the row landed would
  // otherwise be drawn nowhere at all.
  const hasMessages = messages.length > 0 || liveTimelineShown || thinking;
  // While the live answer is on screen, it is the newest answer, so the
  // transcript must not accent one of its own rows as well (spec:
  // chat-thread-layout, "Only the newest answer is accented").
  const liveTimelineHasProse = roundProse.length > 0 || liveText.length > 0;
  // Mid-run projection snapshots can already contain completed tool rounds
  // while the live feed still owns the same run. The live pill is the one
  // source during that overlap; the durable pill appears only when it retires.
  const liveActivityPillShown = liveTimelineShown && (activity.length > 0 || (showReasoning && reasoning.length > 0));

  const composer = (
    <ComposerDock
      activeProjectName={activeProjectName}
      conversationId={conversationId ?? reservedConversationId}
      draftOwner={conversationId}
      disabled={disabled}
      mcpServers={mcpServers}
      mcpServerId={mcpServerId}
      mcpSelectionLoading={mcpSelectionLoading}
      mcpSelectionError={mcpSelectionError}
      mcpSelectionDisabled={mcpSelectionDisabled}
      onMcpServerSelect={onMcpServerSelect}
      onAttach={onAttach}
      onPasteFiles={onPasteFiles}
      attachmentPending={attachmentPending}
      onCancelUpload={onCancelUpload}
      onClearProject={onClearProject}
      onRetryUpload={onRetryUpload}
      onSlashCommand={onSlashCommand}
      onStop={onStop}
      stopping={stopping}
      onSubmit={onSubmit}
      projectFiles={projectFiles}
      skills={skills}
      submitting={submitting}
      streaming={streaming}
    />
  );

  // One card for one death. A run that dies raises BOTH an error (the
  // specific account — it names what ran out) and a recovery notice (the
  // generic "you can retry"); reporting them separately told the reader the
  // same thing twice, in two voices. The error wins when both are present.
  const noticeMessage = error ?? recoveryNotice ?? null;
  const notice = noticeMessage ? (
    <RunNotice
      message={noticeMessage}
      tone={error ? "error" : "notice"}
      onDismiss={onDismissNotice}
      onRetry={onRetryRun}
      retrying={retryingRun}
    />
  ) : null;
  const projectNotice = projectUnavailable ? (
    <Alert tone="warn" title="Project folder unavailable">
      This chat remains readable, but local project tools are off. Reconnect the drive or restore the folder to its
      original location.
    </Alert>
  ) : null;

  // Welcome state mirrors xfalcon-web: logo, personalized greeting, the
  // composer centered between them, then the starter grid. Once a thread
  // exists the composer docks to the bottom of the transcript.
  if (!hasMessages) {
    return (
      <section aria-label="Chat" className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
        <div className="chat-column-composer flex h-full w-full flex-col py-6">
          {availabilityStale && !availabilitySyncing ? (
            <p className="type-caption text-secondary" role="status">
              Offline copy — reconnect to sync
            </p>
          ) : null}
          {projectNotice}
          {notice}
          <EmptyConversation
            canSuggest={canSuggest}
            composer={composer}
            greet={conversationId === null}
            viewerName={viewerName}
            onOpenSettings={onOpenSettings}
            onSuggest={onSubmit}
          />
        </div>
      </section>
    );
  }

  // The conversation title lives in the app titlebar (AppShell) — repeating
  // it here boxed the transcript under a second header. The reading surface
  // is just the transcript, then the run in flight, then the composer.
  // Cancelling a run lives in the composer's send/stop control, never in a
  // separate pill (spec: chat-composer).
  return (
    <section aria-label="Chat" className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
      <div className="flex h-full w-full flex-col pb-4 pt-2">
        <div className="chat-column-messages w-full">
          {availabilityStale && !availabilitySyncing ? (
            <p className="type-caption mb-1 text-secondary" role="status">
              Offline copy — reconnect to sync
            </p>
          ) : null}
          {projectNotice}
        </div>
        <MessageFeedbackProvider key={conversationId ?? "new"} conversationId={conversationId}>
          <MessageList
            conversationId={conversationId}
            followStream={followStream}
            messages={messages}
            onEdit={onEdit}
            onRegenerate={onRegenerate}
            reasoning={liveTimelineShown ? NO_SEGMENTS : reasoning}
            reasoningTrimmed={reasoningTrimmed}
            revisionPending={revisionPending}
            suppressCurrentRunActivity={liveActivityPillShown}
            tailHasLatestAnswer={liveTimelineHasProse}
            liveAnswer={liveText.length > 0 ? { text: liveText, streaming: answerStreaming } : null}
            footer={
              followUpAnswer && conversationId && onSubmit ? (
                <FollowUpChips
                  key={`${conversationId}:${followUpAnswer.id}`}
                  conversationId={conversationId}
                  assistantContent={followUpAnswer.text}
                  userContent={lastUserContent}
                  onSend={onSubmit}
                />
              ) : null
            }
            tail={
              // Everything that belongs to the turn in progress, in the scroll
              // flow where that turn lands: the run's own blocks in the order it
              // produced them, then — if it died — the account of why. A notice
              // pinned above the transcript described a failure the reader had to
              // scroll away from to see.
              <>
                {liveTimelineShown ? (
                  <LiveRunTimeline
                    activity={activity}
                    reasoning={reasoning}
                    reasoningTrimmed={reasoningTrimmed}
                    runActive={runActive}
                  />
                ) : thinking && !notice ? (
                  <ThinkingPill />
                ) : null}
                <ToolApprovalPrompt className="mt-2 flex flex-col gap-2" events={activity} runId={streamRunId} />
                {notice}
              </>
            }
          />
        </MessageFeedbackProvider>
        <div className="chat-column-composer composer-dock mt-3 w-full shrink-0">{composer}</div>
      </div>
    </section>
  );
}
