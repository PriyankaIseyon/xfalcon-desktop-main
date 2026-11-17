import { Fragment, memo, useCallback, useMemo, useRef, useState, type ReactNode } from "react";
import { Brain, ChevronRight, TriangleAlert, Waypoints } from "lucide-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "../../components/ai-elements/collapsible";
import { Tool, ToolContent, ToolHeader, ToolInput, ToolOutput } from "../../components/ai-elements/tool";
import { Badge } from "../../ui/Badge";
import { LatticeLoader } from "../../ui/LatticeLoader";
import { cn } from "../../lib/cn";
import type { ChatToolCallView, ToolInvocationEvent } from "../../lib/generated/contracts";
import { SHOW_TOOL_PAYLOADS, SHOW_TOOL_STEP_SUMMARY } from "./activityDisclosure";
import { AssistantMessage } from "./AssistantMessage";
import { LATTICE } from "./latticeStyle";
import { ThinkingStatus } from "./ThinkingStatus";
import { useThinkingPhrase } from "./thinkingPhrase";
import {
  TERMINAL_STATES,
  formatElapsed,
  formatPayload,
  parseToolId,
  stateForDurableCall,
  stateForEvent,
  stepRow,
} from "./toolPresentation";

/** Bubbling signal MessageList listens for: a disclosure inside a
 * virtualized row settled at a new height and the row must re-measure —
 * including SHRINK, which the streaming-growth cache otherwise refuses
 * (spec: chat-tool-trail, virtualization requirement). It carries no
 * height: the listener measures the row when it handles the report, so the
 * same settle reported twice converges instead of two writers disagreeing. */
export const REMEASURE_EVENT = "xf-remeasure";

/**
 * Reports that a disclosure body settled at its final height — however it
 * got there.
 *
 * `animationend` cannot be the trigger, only one of the moments: the global
 * reduced-motion kill switch (`index.css`) sets `animation: none
 * !important`, so for the users who asked for less motion that event never
 * fires, MessageList's grow-only cache never learns the smaller height, and
 * a collapsed disclosure leaves a permanent gap under the turn (spec:
 * chat-motion-system — no consequence may hang off an animation having
 * played). The question asked is therefore "will anything animate", put to
 * the element exactly as Radix's own presence puts it, never to
 * `matchMedia`: a keyframe missing from a build, a platform that drops
 * animations and the jsdom test environment all answer no for reasons a
 * media query cannot see.
 *
 * Radix keeps this node mounted across a toggle and swaps its children, so
 * the toggle arrives as an attribute mutation rather than a remount — hence
 * the observer, whose microtask also lands after the commit that changed
 * the height. Two facts each mean the height is already final: a `hidden`
 * body contributes none, and a state with no animation declared for it is
 * displayed at full size at once. Anything else waits for `animationend`,
 * so an intermediate frame is never measured as though it had settled.
 *
 * Notes that survive the rewrite: a NATIVE listener because React 19's
 * synthetic animation events do not fire under jsdom, and because nested
 * disclosures' animations bubble through this node — every height settle
 * re-measures. A callback ref rather than an effect because the caller
 * holds no node; its React 19 cleanup return is per-attachment, so ONE
 * reporter serves the umbrella and every nested detail panel.
 */
function useDisclosureSettleReporter() {
  return useCallback((node: HTMLElement | null) => {
    if (!node) return;
    // Captured while the body is still attached, because the body cannot
    // dispatch for itself once removed and the row it sat in outlives it.
    const row = node.closest<HTMLElement>("[data-index]") ?? node.parentElement;
    const report = (from: HTMLElement | null) => {
      from?.dispatchEvent(new CustomEvent(REMEASURE_EVENT, { bubbles: true }));
    };
    const onAnimationEnd = () => report(node);
    node.addEventListener("animationend", onAnimationEnd);
    // A detached node answers with the empty string, which is also "nothing
    // will animate" — and nothing to wait for.
    const settled = () => node.hidden || (getComputedStyle(node).animationName || "none") === "none";
    const observer = new MutationObserver(() => {
      if (settled()) report(node);
    });
    observer.observe(node, { attributeFilter: ["data-state", "hidden"] });
    return () => {
      node.removeEventListener("animationend", onAnimationEnd);
      observer.disconnect();
      // React detaches refs before it removes the nodes, so the row still
      // measures its pre-collapse height here; the microtask reads the
      // settled one.
      queueMicrotask(() => report(row));
    };
  }, []);
}

/**
 * The shared grouping shell (spec: chat-tool-trail, "one visual language"):
 * a quiet disclosure card — summary header, chevron, per-call rows in the
 * body. The durable tool trail, the live run card and the reasoning
 * rows all render through this; AI Elements' `Tool` family renders the tool
 * rows and reasoning uses the same row language.
 */
export function ToolActivityShell({
  label,
  leading,
  bareLeading = false,
  open,
  onOpenChange,
  onPointerEnter,
  onPointerLeave,
  className,
  children,
  bodyLabel,
  disclosable = true,
}: {
  label: ReactNode;
  leading?: ReactNode;
  /** Removes the decorative tile around a live spinner. */
  bareLeading?: boolean;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  onPointerEnter?: () => void;
  onPointerLeave?: () => void;
  className?: string;
  children?: ReactNode;
  bodyLabel?: string;
  /** This block has a body to open. False for the waiting state, which is this
   * same block before the run has produced anything to disclose. */
  disclosable?: boolean;
}) {
  const reportSettle = useDisclosureSettleReporter();
  const surface = cn(
    // One family for waiting, working and settled. A waiting chip that is a
    // blurred pill, then a square card named "Working", then a third chip
    // named "Thought process" is three products in one answer. Collapsed this
    // is a pill the width of its words; open it is a rounded sheet the width
    // of the column, same border and fill.
    "tool-activity-shell shrink-0 self-start w-fit max-w-full rounded-full",
    "transition-[border-radius,width] duration-slow ease-panel",
    "data-[state=open]:w-full data-[state=open]:rounded-xl",
    className,
  );
  // Header markup shared verbatim with the disclosable branch. A waiting turn
  // and a working one are the same block at two moments of one run, so they
  // cannot be two boxes that merely resemble each other: the moment content
  // arrives, this box grows a chevron and a body, and nothing about it is
  // redrawn (spec: chat-run-activity, "one block per response").
  const header = (
    <>
      {leading ? (
        <span className={cn("tool-activity-shell__leading", bareLeading && "tool-activity-shell__leading--bare")}>
          {leading}
        </span>
      ) : null}
      <span className="flex min-w-0 items-center gap-2 truncate tracking-[-0.01em]">{label}</span>
    </>
  );

  if (!disclosable) {
    return (
      <div className={surface}>
        <div className="tool-activity-shell__trigger type-caption flex w-full items-center gap-2 text-left font-medium">
          {header}
        </div>
      </div>
    );
  }

  return (
    <Collapsible
      className={cn(surface, "data-[state=open]:w-full")}
      open={open}
      onOpenChange={onOpenChange}
      onPointerEnter={onPointerEnter}
      onPointerLeave={onPointerLeave}
    >
      <CollapsibleTrigger className="tool-activity-shell__trigger group/shell type-caption flex w-full cursor-pointer items-center gap-2 bg-transparent text-left font-medium">
        {header}
        <span className="tool-activity-shell__chevron ml-auto" aria-hidden>
          <ChevronRight
            className="transition-transform duration-base ease-panel group-data-[state=open]/shell:rotate-90"
            size={13}
            strokeWidth={1.8}
          />
        </span>
      </CollapsibleTrigger>
      <CollapsibleContent ref={reportSettle} className="ai-collapsible-content">
        <div className="tool-activity-shell__body">
          <div aria-label={bodyLabel} className="flex flex-col gap-0.5" role="list">
            {children}
          </div>
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}

/**
 * A stretch of the model's own reasoning.
 *
 * Inert plain text: reasoning routinely contains half-finished fenced blocks,
 * and executing one would mint a preview frame out of syntax the model has not
 * finished writing. It is also never an answer — the prose a model chooses to
 * emit renders as a message row through the full markdown pipeline, and the
 * difference between the two is the whole point of keeping them apart (spec:
 * chat-run-activity, "Streamed content renders where its durable form lives").
 */
function ReasoningBlock({ text }: { text: string }) {
  return (
    <div className="tool-activity-row flex flex-col gap-1.5 rounded-md px-2 py-1.5" role="listitem">
      <div className="flex items-center gap-2 text-xs font-medium text-foreground">
        <span className="tool-state-glyph grid h-5 w-5 shrink-0 place-items-center rounded-full text-muted">
          <Brain aria-hidden size={11} strokeWidth={1.8} />
        </span>
        <span>Reasoning</span>
      </div>
      <p className="m-0 whitespace-pre-wrap pl-7 text-xs leading-relaxed text-secondary">{text}</p>
    </div>
  );
}

/** Says that the retention cap dropped the oldest thinking. A body that
 * silently begins mid-sentence reads as a broken model rather than a bound in
 * the app (spec: chat-reasoning-panel). */
function ReasoningTrimmedNotice() {
  return (
    <p className="m-0 py-1 pl-9 text-2xs text-muted" role="listitem">
      Earlier reasoning was dropped to stay within this conversation&rsquo;s limit.
    </p>
  );
}

/**
 * Prose the run wrote on the way to its answer, rather than as its answer:
 * what it said before it reached for a tool.
 *
 * It renders through the same markdown pipeline as an answer because it is the
 * same kind of writing and routinely contains lists and code spans — only
 * quieter and one step in, because inside this block it is a step of the work
 * and not the reply.
 */
function NarrationBlock({ text, messageId }: { text: string; messageId: string }) {
  return (
    <div className="narration-step py-1.5 pl-1 text-secondary" role="listitem">
      <AssistantMessage conversationId={null} messageId={messageId} text={text} />
    </div>
  );
}

/**
 * One thing a run produced on the way to its answer.
 *
 * `afterCallCount` is how many tool rows existed when it arrived — the axis
 * `chatReduce` already files reasoning and round prose on. It is what lets one
 * block interleave thinking, narration and tool rows in produced order without
 * a second clock, live and settled alike.
 */
export type RunAside = {
  kind: "reasoning" | "narration";
  text: string;
  afterCallCount: number;
  /** This is the oldest surviving stretch and the retention cap dropped what
   * came before it. Reasoning only. */
  trimmed?: boolean;
  /** Identity for the markdown renderer's per-message caches. Narration only. */
  messageId?: string;
};

/** Shared empty default, so a block without asides does not remount its body
 * on every render behind a fresh array literal. */
const NO_ASIDES: readonly RunAside[] = [];

function AsideBlock({ aside }: { aside: RunAside }) {
  if (aside.kind === "narration") {
    return <NarrationBlock messageId={aside.messageId ?? "narration"} text={aside.text} />;
  }
  return (
    <>
      {aside.trimmed ? <ReasoningTrimmedNotice /> : null}
      <ReasoningBlock text={aside.text} />
    </>
  );
}

/**
 * The block's body: every aside and every tool row in the order the run
 * produced them.
 *
 * An aside captured after N rows is drawn before row N, which is where it
 * happened. Anything filed past the last row lands at the end, so a stretch of
 * thinking that arrived after the final tool still reads after it rather than
 * being dropped for having nowhere to go.
 */
function runBlockBody(asides: readonly RunAside[], rowCount: number, renderRow: (index: number) => ReactNode) {
  const out: ReactNode[] = [];
  let cursor = 0;
  const drainThrough = (callCount: number) => {
    while (cursor < asides.length && asides[cursor].afterCallCount <= callCount) {
      out.push(<Fragment key={`aside-${cursor}`}>{<AsideBlock aside={asides[cursor]} />}</Fragment>);
      cursor += 1;
    }
  };
  for (let index = 0; index < rowCount; index += 1) {
    drainThrough(index);
    out.push(renderRow(index));
  }
  drainThrough(Number.MAX_SAFE_INTEGER);
  return out;
}

/**
 * What a finished block is called — ONE vocabulary, so the block that just
 * settled and the block the transcript redraws on reload cannot describe the
 * same work in two ways (spec: chat-run-activity, "the settle is invisible").
 */
function settledBlockLabel(asideCount: number, toolCount: number): string {
  const tools = toolCount === 1 ? "1 tool used" : `${toolCount} tools used`;
  if (asideCount > 0 && toolCount > 0) return `Thought process · ${tools}`;
  if (asideCount > 0) return "Thought process";
  return tools;
}

/**
 * The durable tool umbrella (spec: chat-tool-trail): ONE collapsed summary
 * for a contiguous run of tool rows. Assistant prose closes it and renders as
 * a message row, so what is inside is calls and nothing a durable message
 * carries. Expanded call rows open into Parameters/Result panels fed by the
 * projected payloads (spec: chat-tool-call-detail; capped Rust-side, rendered
 * inert).
 */
export const DurableToolTrail = memo(function DurableToolTrail({
  calls,
  asides = NO_ASIDES,
}: {
  calls: readonly ChatToolCallView[];
  /** The run's thinking and its narration, in produced order. */
  asides?: readonly RunAside[];
}) {
  const [open, setOpen] = useState(false);
  // A call row's Parameters/Result panel is its own uncontrolled disclosure —
  // the shell never sees its open state, and with motion off nothing bubbles
  // out of it — so it reports its own settle (spec: chat-motion-system, "a
  // nested disclosure settles its row too"). One reporter serves every row:
  // the ref's cleanup is per attached node.
  const reportSettle = useDisclosureSettleReporter();
  const hasCalls = calls.length > 0;
  const hasAsides = asides.length > 0;

  if (!hasCalls && !hasAsides) return null;

  const unanswered = calls.filter((call) => call.status === "unanswered").length;

  return (
    <ToolActivityShell
      bodyLabel="Run activity"
      label={
        <>
          <span>{settledBlockLabel(asides.length, calls.length)}</span>
          {unanswered > 0 ? (
            <span className="flex items-center gap-1 text-warn">
              <TriangleAlert aria-hidden size={12} />
              <span className="text-2xs">{unanswered} without a recorded result</span>
            </span>
          ) : null}
        </>
      }
      leading={
        hasAsides ? (
          <Brain aria-hidden className="shrink-0" size={13} strokeWidth={1.8} />
        ) : (
          <Waypoints aria-hidden className="shrink-0" size={13} strokeWidth={1.8} />
        )
      }
      open={open}
      onOpenChange={setOpen}
    >
      {runBlockBody(asides, calls.length, (index) => {
        const call = calls[index];
        // Computed either way, so the formatter and both panels keep a live call
        // site and the disclosure stays one flag away rather than one rewrite.
        const parameters = SHOW_TOOL_PAYLOADS ? formatPayload(call.arguments) : null;
        const result = SHOW_TOOL_PAYLOADS ? formatPayload(call.result) : null;
        const hasDetail = parameters !== null || result !== null;
        const truncated = truncationLabel(call.arguments_truncated, call.result_truncated);
        return (
          <div key={call.call_id} role="listitem">
            <Tool>
              <ToolHeader
                hasDetail={hasDetail}
                meta={truncated ? <Badge tone="warn">{truncated}</Badge> : null}
                state={stateForDurableCall(call)}
                // The step is the row's summary and must be legible without
                // opening it: the reason this trail still cost a JSON parse to
                // read is that the only thing a row said out loud was an
                // identifier.
                step={SHOW_TOOL_STEP_SUMMARY ? stepRow(call.step) : null}
                title={call.name}
              />
              {hasDetail ? (
                <ToolContent ref={reportSettle}>
                  {parameters !== null ? <ToolInput input={parameters} truncated={call.arguments_truncated} /> : null}
                  {result !== null ? <ToolOutput output={result} truncated={call.result_truncated} /> : null}
                </ToolContent>
              ) : null}
            </Tool>
          </div>
        );
      })}
    </ToolActivityShell>
  );
});

interface LiveRow {
  invocationId: string;
  title: string;
  serverHint: string | null;
  state: ReturnType<typeof stateForEvent>;
  errorText: string | null;
  elapsedLabel: string | null;
  truncationLabel: string | null;
}

function truncationLabel(argumentsTruncated: boolean, resultTruncated: boolean): string | null {
  if (argumentsTruncated && resultTruncated) return "Preview shortened";
  if (argumentsTruncated) return "Input preview shortened";
  if (resultTruncated) return "Output preview shortened";
  return null;
}

/**
 * The live run block (spec: chat-run-activity): ONE disclosure holding
 * everything a run in flight produces on the way to its answer — its thinking,
 * its narration and its tool rows — in the order it produced them.
 *
 * One, not one per stretch. A reader watching a response arrive was shown a
 * waiting chip, then a reasoning disclosure, then a narration row, then a tool
 * card: four pieces of chrome for one answer, each naming itself differently,
 * which read as four apps rather than one response. The answer itself stays
 * outside this block, because that is the thing being waited for.
 *
 * Closed until the reader opens it, whatever the run is doing: it is progress
 * to glance at, not something that unfolds by itself and pushes the answer
 * down. Each row's elapsed time is client-observed and advances only with
 * event/render activity (frontend privilege boundary); the header's stopwatch
 * is display only, driven by `ThinkingStatus`.
 */
export const LiveRunActivity = memo(function LiveRunActivity({
  events,
  runActive,
  asides = NO_ASIDES,
  className,
}: {
  events: readonly ToolInvocationEvent[];
  /** The run is still working. */
  runActive: boolean;
  /** The run's thinking and narration so far, in produced order. */
  asides?: readonly RunAside[];
  className?: string;
}) {
  const phrase = useThinkingPhrase();
  const hasEvents = events.length > 0;
  const [manualOpen, setManualOpen] = useState<boolean | null>(null);
  // A failed row's error panel is the live card's nested disclosure; same
  // reasoning as the durable trail's detail panels.
  const reportSettle = useDisclosureSettleReporter();

  // Client-observed timing, advanced only by event arrival.
  const timingRef = useRef(new Map<string, { firstMs: number; lastMs: number }>());

  const rows = useMemo<LiveRow[]>(() => {
    const order: string[] = [];
    const toolById = new Map<string, string>();
    const latestByInvocation = new Map<string, ToolInvocationEvent>();
    events.forEach((event) => {
      const id = event.invocation_id;
      if (!latestByInvocation.has(id)) {
        order.push(id);
      }
      latestByInvocation.set(id, event);
      if ((event.type === "requested" || event.type === "approval_required") && !toolById.has(id)) {
        toolById.set(id, event.tool_id);
      }
      const now = Date.now();
      const timing = timingRef.current.get(id);
      if (!timing) {
        timingRef.current.set(id, { firstMs: now, lastMs: now });
      } else {
        timing.lastMs = now;
      }
    });
    return order.map((invocationId) => {
      const latest = latestByInvocation.get(invocationId) as ToolInvocationEvent;
      const state = stateForEvent(latest);
      const { title, serverHint } = parseToolId(toolById.get(invocationId) ?? "Unknown tool");
      const errorText =
        latest.type === "failed" ? latest.message : latest.type === "denied" ? `Denied: ${latest.reason_code}` : null;
      const timing = timingRef.current.get(invocationId);
      const elapsedLabel = timing
        ? formatElapsed((TERMINAL_STATES.has(state) ? timing.lastMs : Date.now()) - timing.firstMs)
        : null;
      const rowTruncationLabel = latest.type === "completed" && latest.truncated ? "Output shortened" : null;
      return {
        invocationId,
        title,
        serverHint,
        state,
        errorText,
        elapsedLabel,
        truncationLabel: rowTruncationLabel,
      };
    });
  }, [events]);

  const hasAsides = asides.length > 0;

  // A block with no rows and nothing to say has nothing to disclose. Refused
  // here rather than by the caller.
  if (!hasEvents && !hasAsides) return null;

  const failed = rows.some((row) => row.state === "output-error");

  // While the run works, the header reports that the app is working and never
  // what the run is doing: the run's state is the body, one expand away, and a
  // header that tracked it renamed itself several times inside one answer
  // (spec: chat-run-activity, "one vocabulary"). Once it settles, the label is
  // the same one the transcript will redraw on reload, so the settle changes
  // nothing the reader can see.
  const label = runActive ? (
    <ThinkingStatus phrase={phrase} />
  ) : failed ? (
    "Tool run failed"
  ) : (
    settledBlockLabel(asides.length, rows.length)
  );

  // Closed until the reader opens it: the card is progress to glance at, never
  // something that unfolds on its own and pushes the answer down.
  const open = manualOpen ?? false;
  return (
    <ToolActivityShell
      bodyLabel="Run activity"
      bareLeading={runActive}
      className={className}
      label={<span className={cn(failed && !runActive && "text-danger")}>{label}</span>}
      leading={
        runActive ? (
          <LatticeLoader className="tool-activity-shell__spinner" {...LATTICE} />
        ) : hasAsides ? (
          <Brain aria-hidden className="shrink-0 text-muted" size={12} strokeWidth={1.8} />
        ) : (
          <span className="h-[5px] w-[5px] shrink-0 rounded-full bg-faint" />
        )
      }
      open={open}
      onOpenChange={(next) => setManualOpen(next)}
    >
      {runBlockBody(asides, rows.length, (index) => {
        const row = rows[index];
        // The row still shows its failed STATE, and the block's own label
        // still says the run failed; only the reason is withheld, because a
        // tool's error text routinely quotes what was sent to it.
        const hasDetail = SHOW_TOOL_PAYLOADS && row.errorText !== null;
        return (
          <div key={row.invocationId} role="listitem">
            <Tool>
              {/* The same row component the durable trail draws, so a step
                  renders identically wherever one exists. A live row has
                  none to pass: its detail contract is
                  `ToolInvocationEvent`, whose terminal event carries a
                  lifecycle status and no outcome document — the step
                  appears the moment the round becomes durable. */}
              <ToolHeader
                hasDetail={hasDetail}
                meta={
                  row.truncationLabel || row.elapsedLabel ? (
                    <>
                      {row.truncationLabel ? <Badge tone="warn">{row.truncationLabel}</Badge> : null}
                      {row.elapsedLabel ? <span className="text-2xs text-muted">{row.elapsedLabel}</span> : null}
                    </>
                  ) : null
                }
                serverHint={row.serverHint}
                state={row.state}
                title={row.title}
              />
              {hasDetail ? (
                <ToolContent ref={reportSettle}>
                  <ToolOutput errorText={row.errorText} />
                </ToolContent>
              ) : null}
            </Tool>
          </div>
        );
      })}
    </ToolActivityShell>
  );
});
