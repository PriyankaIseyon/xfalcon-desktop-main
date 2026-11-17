import { useId, useLayoutEffect, useMemo, useState } from "react";
import type { ToolInvocationEvent } from "../../lib/generated/contracts";
import { chatToolApprovalResolve } from "../../lib/native";
import { ToolStateGlyph } from "../../components/ai-elements/tool";
import { Button } from "../../ui/Button";
import { parseToolId } from "../chat/toolPresentation";

export interface ToolApprovalPromptProps {
  /** The run's live invocation feed. */
  events: readonly ToolInvocationEvent[];
  /** The run those events belong to, or `null` when no run is active. */
  runId: string | null;
  className?: string;
}

type ApprovalRequiredEvent = Extract<ToolInvocationEvent, { type: "approval_required" }>;
type Decision = "allow" | "deny";

function ApprovalDecision({ event, runId }: { event: ApprovalRequiredEvent; runId: string }) {
  const titleId = useId();
  const descriptionId = useId();
  const allowId = useId();
  const denyId = useId();
  const [resolving, setResolving] = useState<Decision | null>(null);
  const [submitted, setSubmitted] = useState<Decision | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [failedDecision, setFailedDecision] = useState<Decision | null>(null);
  const { title, serverHint } = parseToolId(event.tool_id);

  // The busy state disables the active button, which lets a browser move
  // focus away. Restore it only after React has committed the enabled failure
  // state, so a keyboard retry continues at the decision that failed.
  useLayoutEffect(() => {
    if (error === null || failedDecision === null) return;
    document.getElementById(failedDecision === "allow" ? allowId : denyId)?.focus();
  }, [allowId, denyId, error, failedDecision]);

  const resolve = async (decision: Decision) => {
    if (resolving !== null || submitted !== null) return;
    setResolving(decision);
    setError(null);
    setFailedDecision(null);
    try {
      await chatToolApprovalResolve(runId, event.invocation_id, decision === "allow");
      // The command accepted the decision. Keep both controls unavailable
      // until the lifecycle event removes this prompt, so a delayed event
      // cannot turn one decision into two backend calls.
      setSubmitted(decision);
      setResolving(null);
    } catch (cause) {
      const detail = cause instanceof Error ? cause.message : String(cause);
      setError(`The decision could not be saved. ${detail}`);
      setFailedDecision(decision);
      setResolving(null);
    }
  };

  const unavailable = resolving !== null || submitted !== null;

  return (
    <section
      aria-busy={resolving !== null || undefined}
      aria-describedby={descriptionId}
      aria-labelledby={titleId}
      className="flex flex-col gap-2 rounded-lg border border-warn bg-warn-bg px-3 py-2"
    >
      <div className="flex items-start gap-2">
        <span className="mt-0.5 shrink-0">
          <ToolStateGlyph state="approval-requested" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-xs font-medium text-foreground" id={titleId}>
            Approval required for <span className="break-all font-mono">{title}</span>
            {serverHint ? <span className="text-muted"> · {serverHint}</span> : null}
          </p>
          <p className="mt-0.5 text-2xs leading-normal text-secondary" id={descriptionId}>
            This tool can change things outside the conversation. Nothing runs until you decide.
          </p>
        </div>
      </div>
      {error ? (
        <p className="text-2xs text-danger" role="alert">
          {error}
        </p>
      ) : submitted ? (
        <p className="text-2xs text-secondary" role="status">
          Decision sent. Waiting for the run to continue.
        </p>
      ) : null}
      <div
        aria-label={`Decision for ${title}`}
        className="flex flex-wrap items-center justify-end gap-1.5"
        role="group"
      >
        <Button
          id={denyId}
          disabled={unavailable}
          loading={resolving === "deny"}
          size="sm"
          variant="secondary"
          onClick={() => {
            void resolve("deny");
          }}
        >
          {resolving === "deny" ? "Denying…" : submitted === "deny" ? "Denied" : "Deny"}
        </Button>
        <Button
          id={allowId}
          disabled={unavailable}
          loading={resolving === "allow"}
          size="sm"
          variant="primary"
          onClick={() => {
            void resolve("allow");
          }}
        >
          {resolving === "allow" ? "Allowing…" : submitted === "allow" ? "Allowed" : "Allow once"}
        </Button>
      </div>
    </section>
  );
}

/**
 * Approves or declines a tool call the agent run has paused on.
 *
 * Distinct from `ApprovalSheet`, which reviews a *file mutation plan*. This is
 * the generic tool gate: `ToolLoop` pauses any invocation of a tool the
 * registry marks `mutating`, and until someone decides, the run does not
 * continue. Both surfaces exist because they answer different questions —
 * "may this tool run at all" versus "are these exact file edits correct".
 *
 * Rendered as web's compact awaiting-approval card (`ToolCallGroup`): a
 * quiet amber-edged inline card in the conversation column, not a
 * full-width banner shouting over the transcript.
 *
 * Derived from the same journaled event stream the activity feed renders, so
 * it can never show a pending decision the backend does not actually hold: an
 * `approval_required` event that is later followed by any other event for the
 * same invocation is no longer pending.
 */
export function ToolApprovalPrompt({ events, runId, className }: ToolApprovalPromptProps) {
  const pending = useMemo(() => {
    const latest = new Map<string, ToolInvocationEvent>();
    for (const event of events) {
      latest.set(event.invocation_id, event);
    }
    return [...latest.values()].filter((event): event is ApprovalRequiredEvent => event.type === "approval_required");
  }, [events]);

  if (runId == null || pending.length === 0) return null;

  return (
    <div aria-label="Tool approvals" className={className} role="group">
      {pending.map((event) => (
        <ApprovalDecision event={event} key={`${runId}:${event.invocation_id}`} runId={runId} />
      ))}
    </div>
  );
}
