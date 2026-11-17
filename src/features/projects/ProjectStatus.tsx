import React from "react";
import { cn } from "../../lib/cn";
import { Tooltip } from "../../ui/Tooltip";
import { StatusDot, type StatusDotTone } from "../../ui/StatusDot";

export type ProjectIndexState = "loading" | "ready" | "error";

export interface ProjectIndexStatus {
  state: ProjectIndexState;
  /** Human-readable detail -- phase/file counts for loading, the failure reason for error. */
  detail?: string;
}

export interface ProjectStatusProps {
  status: ProjectIndexStatus;
  className?: string;
  /**
   * Set to `false` when embedding inside another interactive control (e.g. a
   * menu item, which is itself a `<button>`) where a nested, click-to-expand
   * `<button>` would be invalid HTML. In that mode the indicator is a plain
   * labelled dot: error detail still surfaces automatically, and loading
   * detail arrives on hover instead of on a disclosure. Defaults to `true` (a
   * focusable, expandable indicator).
   */
  interactive?: boolean;
}

const STATE_META: Record<ProjectIndexState, { tone: StatusDotTone; pulse: boolean; label: string }> = {
  loading: { tone: "busy", pulse: true, label: "Indexing project" },
  ready: { tone: "online", pulse: false, label: "Index ready" },
  error: { tone: "error", pulse: false, label: "Project unavailable" },
};

/**
 * Presentational, quiet-by-default indicator for a project's background
 * index state. `status` is a prop the caller controls -- this component has
 * no polling or side effects of its own (there is no live indexer wired to
 * any Tauri command yet; the parent decides what value to pass).
 *
 * In the common "loading"/"ready" states it renders only a dot, so it never
 * competes with chat for attention. When `status.state` is "error" it
 * automatically surfaces the detail text, because that is the one state
 * worth interrupting for; the user can also expand loading's detail on
 * demand via the dot itself.
 */
export function ProjectStatus({ status, className, interactive = true }: ProjectStatusProps) {
  const [expanded, setExpanded] = React.useState(false);
  const meta = STATE_META[status.state];
  const isError = status.state === "error";
  const canExpand = interactive && Boolean(status.detail) && !isError;
  const showDetail = Boolean(status.detail) && (isError || (interactive && expanded));
  const detailId = React.useId();
  const dot = <StatusDot label={meta.label} pulse={meta.pulse} tone={meta.tone} />;

  // A hover hint only where the detail is not already on screen: in the error
  // state the paragraph below says the same words out loud.
  const quiet = <span className="flex w-fit items-center gap-1.5 p-0.5">{dot}</span>;
  const quietIndicator = status.detail && !showDetail ? <Tooltip label={status.detail}>{quiet}</Tooltip> : quiet;

  const indicator = canExpand ? (
    <button
      aria-controls={showDetail ? detailId : undefined}
      aria-expanded={expanded}
      aria-label={meta.label}
      className="press flex w-fit cursor-pointer items-center gap-1.5 rounded-md bg-transparent p-0.5"
      onClick={() => setExpanded((current) => !current)}
      type="button"
    >
      {dot}
    </button>
  ) : (
    quietIndicator
  );

  return (
    <div className={cn("flex flex-col gap-1", className)}>
      {indicator}
      {showDetail ? (
        <p
          className={cn("type-caption pl-0.5", isError ? "text-danger" : "text-muted")}
          id={detailId}
          role={isError ? "alert" : "status"}
        >
          {status.detail}
        </p>
      ) : null}
    </div>
  );
}
