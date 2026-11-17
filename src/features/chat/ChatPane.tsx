import { ChatErrorBoundary } from "./ChatErrorBoundary";
import { ChatSurface } from "./ChatSurface";
import { useTranscript } from "./useTranscript";
import { ConversationSkeleton } from "./ConversationSkeleton";

export type ChatPaneProps = {
  /** The secure runtime is unavailable — typing and sending are both off. */
  disabled: boolean;
  /** Display name of the trusted project a submitted message runs against. */
  activeProjectName: string | null;
  /** Leaves project context and opens a fresh ordinary chat. */
  onClearProject: () => void;
  /** The bound project exists but its trusted folder cannot be reopened. */
  projectUnavailable: boolean;
  viewerName: string | null;
  onOpenSettings: () => void;
  onSubmit: (prompt: string, attachmentIds?: string[]) => void;
  onEdit?: (messageId: string, text: string) => void;
  onRegenerate?: (messageId: string) => void;
  onStop: () => void;
  /** A Stop press is in flight for the run on screen. */
  stopping: boolean;
  onRetryRun: () => void;
  onDismissNotice: () => void;
  onResetConversation: () => void;
  projectFiles?: string[];
  mcpServers?: string[];
  mcpServerId?: string | null;
  mcpSelectionLoading?: boolean;
  mcpSelectionError?: string | null;
  mcpSelectionDisabled?: boolean;
  onMcpServerSelect?: (serverId: string | null) => void;
  skills?: Array<{ name: string; description: string }>;
  onSlashCommand?: (command: string) => void;
  onAttach?: () => void;
  onPasteFiles?: (files: File[]) => Promise<void>;
  attachmentPending?: boolean;
  onCancelUpload?: (uploadId: string) => void;
  onRetryUpload?: (uploadId: string) => void;
};

/**
 * The chat column, and the only component subscribed to per-token state.
 *
 * It exists for that reason alone. `AppShell` used to read the whole chat
 * snapshot, so every streamed token re-rendered the entire shell — the sidebar,
 * the titlebar, the command palette, all of it — to grow one paragraph. The
 * shell now holds the actions and the conversation-scoped fields, and everything
 * a token can move is read here (spec: chat-streaming-performance).
 */
export function ChatPane({
  disabled,
  activeProjectName,
  onClearProject,
  projectUnavailable,
  viewerName,
  onOpenSettings,
  onSubmit,
  onEdit,
  onRegenerate,
  onStop,
  stopping,
  onRetryRun,
  onDismissNotice,
  onResetConversation,
  projectFiles,
  mcpServers,
  mcpServerId,
  mcpSelectionLoading,
  mcpSelectionError,
  mcpSelectionDisabled,
  onMcpServerSelect,
  skills,
  onSlashCommand,
  onAttach,
  onPasteFiles,
  attachmentPending,
  onCancelUpload,
  onRetryUpload,
}: ChatPaneProps) {
  const transcript = useTranscript();
  // One derived turn contract feeds every consumer. The accepted run owns the
  // turn from its first known id through a terminal lifecycle state; submission
  // and revision claims cover the intervals before that id exists. Keeping
  // these booleans together prevents the pill, composer, scroll follower, and
  // focus handoff from observing different definitions of "working".
  const runActive = transcript.pending;
  const submissionActive = transcript.awaitingRun || transcript.revising;
  const turnActive = runActive || submissionActive;

  return (
    <div className="app-chat-column" aria-busy={transcript.conversationContentPending || undefined}>
      <ChatErrorBoundary resetKey={transcript.conversationId} onReset={onResetConversation}>
        {transcript.conversationContentPending ? (
          <ConversationSkeleton />
        ) : (
          <ChatSurface
            activeProjectName={activeProjectName}
            activity={transcript.activity}
            answerStreaming={transcript.streaming}
            availabilityStale={transcript.availability === "stale"}
            availabilitySyncing={transcript.conversationSyncing}
            conversationId={transcript.conversationId}
            reservedConversationId={transcript.reservedConversationId}
            disabled={disabled}
            error={transcript.error}
            followStream={transcript.streaming || turnActive}
            liveText={transcript.liveText}
            mcpServers={mcpServers}
            mcpServerId={mcpServerId}
            mcpSelectionLoading={mcpSelectionLoading}
            mcpSelectionError={mcpSelectionError}
            mcpSelectionDisabled={
              mcpSelectionDisabled ||
              turnActive ||
              transcript.conversationSyncing ||
              transcript.conversationContentPending ||
              attachmentPending
            }
            onMcpServerSelect={onMcpServerSelect}
            messages={transcript.messages}
            onDismissNotice={onDismissNotice}
            onClearProject={onClearProject}
            onEdit={onEdit}
            onOpenSettings={onOpenSettings}
            onRegenerate={onRegenerate}
            // Retry replays a specific run; without one bound to this view (the
            // startup banner's case) the button would no-op.
            onRetryRun={transcript.activeRunId && transcript.recoveryNotice ? onRetryRun : undefined}
            onSlashCommand={onSlashCommand}
            onAttach={onAttach}
            onPasteFiles={onPasteFiles}
            attachmentPending={attachmentPending}
            onCancelUpload={onCancelUpload}
            onRetryUpload={onRetryUpload}
            onStop={onStop}
            stopping={stopping}
            onSubmit={onSubmit}
            projectFiles={projectFiles}
            skills={skills}
            projectUnavailable={projectUnavailable}
            reasoning={transcript.reasoning}
            reasoningTrimmed={transcript.reasoningTrimmed}
            recoveryNotice={transcript.recoveryNotice}
            retryingRun={transcript.awaitingRun}
            revisionPending={transcript.revising}
            roundProse={transcript.roundProse}
            runActive={runActive}
            streamRunId={transcript.streamRunId}
            streaming={runActive}
            submitting={submissionActive}
            // The pill means "in flight and nothing it produced is on screen".
            // Whether anything IS on screen is the live timeline's own predicate,
            // read by `ChatSurface` — asking it here from a different set of
            // conditions is how the pill came to be suppressed by reasoning the
            // surface was not drawing.
            //
            // `awaitingRun` covers the turn between the reader pressing send and
            // the backend answering with a run id: the run is a second or more
            // away, and only the indicator runs ahead of it. Nothing that acts on
            // a run — the stop control, the activity card — may, because there is
            // no run to act on yet.
            thinking={turnActive}
            viewerName={viewerName}
          />
        )}
      </ChatErrorBoundary>
    </div>
  );
}
