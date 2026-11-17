/**
 * Vendored from Vercel AI Elements (`https://registry.ai-sdk.dev/tool.json`,
 * Apache-2.0) and aligned to this app's visual system. Divergences from the
 * upstream source, each forced by a desktop constraint:
 *
 * - `ToolUIPart` (AI SDK type) → local `ToolRowState`: this app's tool data
 *   is `ToolInvocationEvent` / `ChatToolCallView`, not AI SDK message parts,
 *   and the union carries two desktop-only states (`cancelled`,
 *   `no-result`).
 * - shadcn `Badge`/`cn`/CSS vars → `src/ui/Badge` and `--xf-*` tokens.
 * - shadcn `CodeBlock` (shiki) → local `.code-surface` <pre>: payloads are
 *   untrusted model/tool output and render as inert text nodes only.
 * - tailwindcss-animate classes → `.ai-collapsible-content` keyframes on
 *   Radix's `--radix-collapsible-content-height` (reduced-motion safe via
 *   the global kill switch).
 * - Rows are borderless: grouping chrome belongs to `ToolActivityCard`
 *   (spec chat-tool-trail: one grouped disclosure per turn, never a stack
 *   of bordered cards).
 */
import { Check, ChevronRight, Circle, OctagonX, ShieldAlert, ShieldQuestion, TriangleAlert, X } from "lucide-react";
import type { ComponentProps, ReactNode } from "react";
import { Badge, type BadgeTone } from "../../ui/Badge";
import { Chip } from "../../ui/Chip";
import { Tooltip } from "../../ui/Tooltip";
import { Spinner } from "../../ui/Spinner";
import { cn } from "../../lib/cn";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "./collapsible";

/** Lifecycle presentation states, superset of AI Elements' `ToolUIPart["state"]`
 * (`input-streaming` / `input-available` / `approval-requested` /
 * `output-available` / `output-error` / `output-denied`) plus the desktop
 * extras `cancelled` and `no-result` (durable trail rows have no recorded
 * answer to show — spec: chat-tool-trail). */
export type ToolRowState =
  | "input-streaming"
  | "input-available"
  | "approval-requested"
  | "output-available"
  | "output-error"
  | "output-denied"
  | "cancelled"
  | "no-result";

const STATE_LABEL: Record<ToolRowState, string> = {
  "input-streaming": "Pending",
  "input-available": "Running",
  "approval-requested": "Awaiting approval",
  "output-available": "Completed",
  "output-error": "Failed",
  "output-denied": "Denied",
  cancelled: "Cancelled",
  "no-result": "No result recorded",
};

const STATE_TONE: Record<ToolRowState, BadgeTone> = {
  "input-streaming": "neutral",
  "input-available": "accent",
  "approval-requested": "warn",
  "output-available": "success",
  "output-error": "danger",
  "output-denied": "danger",
  cancelled: "neutral",
  "no-result": "warn",
};

/** 12px glyph in a fixed slot so status transitions never shift the row. */
export function ToolStateGlyph({ state }: { state: ToolRowState }) {
  const icon: Record<ToolRowState, ReactNode> = {
    "input-streaming": <Circle aria-hidden className="text-faint" size={12} strokeWidth={2.2} />,
    "input-available": <Spinner decorative className="text-accent-hover" size={12} />,
    "approval-requested": <ShieldQuestion aria-hidden className="text-warn" size={12} strokeWidth={2.2} />,
    "output-available": <Check aria-hidden className="text-success" size={12} strokeWidth={2.2} />,
    "output-error": <X aria-hidden className="text-danger" size={12} strokeWidth={2.2} />,
    "output-denied": <OctagonX aria-hidden className="text-danger" size={12} strokeWidth={2.2} />,
    cancelled: <X aria-hidden className="text-muted" size={12} strokeWidth={2.2} />,
    "no-result": <TriangleAlert aria-hidden className="text-warn" size={12} strokeWidth={2.2} />,
  };
  return (
    <span
      key={state}
      className="tool-state-glyph badge-pop grid h-5 w-5 shrink-0 place-items-center rounded-full"
      data-state={state}
    >
      {icon[state]}
    </span>
  );
}

export type ToolProps = ComponentProps<typeof Collapsible>;

/** One tool call: a collapsible row. Chrome-free — the grouping card owns
 * borders and background. */
export const Tool = ({ className, ...props }: ToolProps) => (
  <Collapsible className={cn("w-full", className)} {...props} />
);

/** One file a step touched, as a row draws it: the value, the shortened form,
 * and the classifier's verdict (`src/features/chat/toolPresentation.ts`). */
export type ToolStepPathRow = {
  /** The full project-relative path — the chip's accessible name. */
  path: string;
  /** The same value shortened to fit the row. */
  display: string;
  sensitive: boolean;
};

/** What a call did, as the projection composed it. Never composed here. */
export type ToolStepRow = {
  label: string;
  paths: readonly ToolStepPathRow[];
  /** Stated when fewer paths are shown than the run touched. */
  boundNote: string | null;
};

/**
 * A step's path: a bounded, wrapping, NON-INTERACTIVE datum.
 *
 * Inert by rule, not by taste — plain text nodes only, no markdown, no markup,
 * no linkification, no click target. A path reaches this surface from a durable
 * row that travelled the data plane, and the projection carries it as display
 * evidence; giving it a click target would promote that value into an argument
 * to a file operation, which belongs to the trusted-root resolution `projects`
 * owns and not to a render function.
 *
 * A sensitive path carries a glyph AND says so in its accessible name, because
 * the classifier's whole purpose is that a person can see which files were
 * withheld — and colour alone does not tell them.
 */
function ToolStepPathChip({ path }: { path: ToolStepPathRow }) {
  // The visible text is the accessible name whenever the two are the same
  // string; a spoken name is added only when they differ — because the value was
  // shortened, or because "sensitive" is part of what this chip says and a glyph
  // does not say it.
  const spoken = path.sensitive ? `Sensitive file: ${path.path}` : path.display === path.path ? null : path.path;
  return (
    <Tooltip label={path.path}>
      <Chip
        className={cn(
          "gap-1 rounded-md border-hairline px-1.5 py-0.5 font-mono text-2xs",
          path.sensitive && "border-warn/40 bg-warn-bg text-warn",
        )}
        interactive={false}
      >
        {path.sensitive ? <ShieldAlert aria-hidden className="shrink-0" size={11} /> : null}
        {spoken ? <span className="sr-only">{spoken}</span> : null}
        <span aria-hidden={spoken ? true : undefined} className="truncate">
          {path.display}
        </span>
      </Chip>
    </Tooltip>
  );
}

export type ToolHeaderProps = {
  /** Human display name (already parsed/formatted by the caller). */
  title: string;
  /** Quiet origin hint rendered after the title (e.g. the MCP server). */
  serverHint?: string | null;
  state: ToolRowState;
  /** Trailing meta (elapsed time). */
  meta?: ReactNode;
  /** Rows with nothing to disclose render a static header — no chevron,
   * no toggle (web `ToolCallGroup` parity). */
  hasDetail?: boolean;
  /** What the call did, when the projection carried a summary. A row without
   * one renders on its name alone, exactly as before. */
  step?: ToolStepRow | null;
  className?: string;
};

export const ToolHeader = ({
  className,
  title,
  serverHint,
  state,
  meta,
  hasDetail = true,
  step = null,
}: ToolHeaderProps) => {
  // The wire name is kept whatever else the row says: a label cannot be matched
  // against a tool catalog, and two tools whose steps read alike are still two
  // tools (spec: chat-tool-trail, "Label and name coexist"). It steps back to
  // secondary ink when a step speaks first.
  const identity = (
    <>
      <span className={cn("truncate font-mono", step ? "text-2xs text-muted" : "text-xs text-foreground")}>
        {title}
      </span>
      {serverHint ? <span className="truncate text-2xs text-faint">· {serverHint}</span> : null}
    </>
  );
  const body = (
    <>
      <ToolStateGlyph state={state} />
      {step ? (
        <span className="flex min-w-0 flex-1 flex-col gap-1">
          <span className="flex min-w-0 items-baseline gap-2">
            <span className="truncate text-2xs font-medium text-foreground">{step.label}</span>
            {identity}
          </span>
          {step.paths.length > 0 ? (
            <span className="flex flex-wrap items-center gap-1">
              {step.paths.map((path) => (
                <ToolStepPathChip key={path.path} path={path} />
              ))}
              {/* Never an ellipsis or a placeholder chip standing in for the
                  difference: the two numbers are said outright. */}
              {step.boundNote ? <span className="text-2xs text-faint">{step.boundNote}</span> : null}
            </span>
          ) : null}
        </span>
      ) : (
        identity
      )}
      <span className="ml-auto flex shrink-0 items-center gap-2">
        {state !== "output-available" ? <Badge tone={STATE_TONE[state]}>{STATE_LABEL[state]}</Badge> : null}
        {meta}
        {hasDetail ? (
          <ChevronRight
            aria-hidden
            className="text-faint transition-transform duration-fast group-data-[state=open]/tool:rotate-90"
            size={12}
          />
        ) : null}
      </span>
    </>
  );
  const rowClass = cn(
    "tool-activity-row flex min-h-8 w-full gap-2 rounded-md px-2 py-1 text-left",
    step ? "items-start py-1.5" : "items-center",
    className,
  );
  if (!hasDetail) {
    return <div className={rowClass}>{body}</div>;
  }
  return (
    <CollapsibleTrigger className={cn(rowClass, "group/tool cursor-pointer hover:bg-hover")}>{body}</CollapsibleTrigger>
  );
};

export type ToolContentProps = ComponentProps<typeof CollapsibleContent>;

export const ToolContent = ({ className, children, ...props }: ToolContentProps) => (
  <CollapsibleContent className={cn("ai-collapsible-content", className)} {...props}>
    <div className="ml-[7px] space-y-2 border-l border-hairline py-1 pl-3">{children}</div>
  </CollapsibleContent>
);

/** Untrusted machine text on an inset surface: inert text nodes only, no
 * markdown, no markup interpretation (upload-cell-injection doctrine). */
function PayloadBlock({ text }: { text: string }) {
  return (
    <pre className="code-surface max-h-56 overflow-x-auto whitespace-pre-wrap break-words p-2.5 text-2xs leading-relaxed">
      {text}
    </pre>
  );
}

/** Explicit disclosure whenever the projection cut a payload (spec:
 * chat-tool-call-detail): the viewer is bounded, the execution log is not. */
const TruncationNote = () => (
  <p className="text-2xs text-faint">Truncated at 32 KiB — the full record stays in the execution log.</p>
);

export type ToolInputProps = ComponentProps<"div"> & {
  /** Pre-formatted text renders verbatim; anything else is stringified. */
  input: unknown;
  truncated?: boolean;
};

export const ToolInput = ({ className, input, truncated = false, ...props }: ToolInputProps) => (
  <div className={cn("space-y-1", className)} {...props}>
    <h4 className="type-eyebrow text-faint">Parameters</h4>
    <PayloadBlock text={typeof input === "string" ? input : (JSON.stringify(input, null, 2) ?? "null")} />
    {truncated ? <TruncationNote /> : null}
  </div>
);

export type ToolOutputProps = ComponentProps<"div"> & {
  output?: unknown;
  errorText?: string | null;
  truncated?: boolean;
};

export const ToolOutput = ({ className, output, errorText, truncated = false, ...props }: ToolOutputProps) => {
  if (output === undefined && !errorText) return null;
  return (
    <div className={cn("space-y-1", className)} {...props}>
      <h4 className={cn("type-eyebrow", errorText ? "text-danger" : "text-faint")}>{errorText ? "Error" : "Result"}</h4>
      {errorText ? (
        <p className="whitespace-pre-wrap break-words rounded-md bg-danger-bg p-2 text-2xs text-danger" role="alert">
          {errorText}
        </p>
      ) : (
        <PayloadBlock text={typeof output === "string" ? output : (JSON.stringify(output, null, 2) ?? "null")} />
      )}
      {truncated ? <TruncationNote /> : null}
    </div>
  );
};
