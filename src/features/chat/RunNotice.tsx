import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";

export type RunNoticeTone = "notice" | "error";

export type RunNoticeProps = {
  message: string;
  /** `error` for a run that died, `notice` for one that can be resumed. */
  tone?: RunNoticeTone;
  onRetry?: () => void;
  onDismiss?: () => void;
  retrying?: boolean;
};

/**
 * The one place a run's failure is reported (spec: chat-run-recovery).
 *
 * Two rules, both learned from the same screenshot:
 *
 * - **One card.** An interrupted run raises a recovery notice AND an
 *   inference failure for the same death. Stacking them reported one event
 *   twice, in two visual languages, with two different accounts of it.
 * - **At the end of the transcript.** The notice belongs beside the turn
 *   that died, which is the last one. Pinned above the scroll region it
 *   described a failure the reader had to scroll away from to see, and it
 *   stayed on screen over every turn that came after.
 */
export function RunNotice({ message, tone = "notice", onRetry, onDismiss, retrying = false }: RunNoticeProps) {
  const isError = tone === "error";
  const actions =
    onRetry || onDismiss ? (
      <div className="flex flex-wrap justify-end gap-2">
        {onRetry ? (
          <Button loading={retrying} size="sm" variant="secondary" onClick={onRetry}>
            {retrying ? "Retrying…" : "Retry run"}
          </Button>
        ) : null}
        {onDismiss ? (
          <Button disabled={retrying} size="sm" variant="ghost" onClick={onDismiss}>
            Dismiss
          </Button>
        ) : null}
      </div>
    ) : undefined;

  return (
    <Alert action={actions} className="mt-2 flex-wrap" tone={isError ? "danger" : "info"}>
      <p className="type-caption">{message}</p>
    </Alert>
  );
}
