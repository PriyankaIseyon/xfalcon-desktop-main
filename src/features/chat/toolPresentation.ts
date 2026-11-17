import type { ChatToolCallView, ToolInvocationEvent, ToolStepSummaryV1 } from "../../lib/generated/contracts";
import type { ToolStepRow, ToolRowState } from "../../components/ai-elements/tool";

/**
 * Naming rules (spec: chat-tool-trail, "Naming does not guess").
 *
 * Live rows carry the internal dotted id (`mcp.<server>.<tool>`,
 * `src-tauri/src/mcp/runtime.rs`) — parsed into tool + server hint. Durable
 * rows carry the sanitized wire name (`mcp_tenant_ida_query`,
 * `tool_session.rs::unique_wire_name`); underscores are ambiguous, so the
 * wire name renders verbatim until `expose-tool-call-detail` supplies the
 * true name.
 */
export function parseToolId(toolId: string): { title: string; serverHint: string | null } {
  if (toolId.startsWith("mcp.")) {
    const rest = toolId.slice("mcp.".length);
    const separator = rest.indexOf(".");
    if (separator > 0 && separator < rest.length - 1) {
      return { title: rest.slice(separator + 1), serverHint: rest.slice(0, separator) };
    }
  }
  return { title: toolId, serverHint: null };
}

/** Mirrors `status_for_event` in `src-tauri/src/agent/tool_loop.rs` — the
 * backend's own lifecycle derivation, mapped onto presentation states. */
export function stateForEvent(event: ToolInvocationEvent): ToolRowState {
  switch (event.type) {
    case "requested":
    case "approved":
      // `approved` is greenlit-but-not-started; presenting it as anything
      // "running" would claim execution that has not begun.
      return "input-streaming";
    case "approval_required":
      return "approval-requested";
    case "denied":
      return "output-denied";
    case "started":
    case "progress":
      return "input-available";
    case "completed":
      return "output-available";
    case "failed":
      return "output-error";
    case "cancelled":
      return "cancelled";
  }
}

export function stateForDurableCall(call: ChatToolCallView): ToolRowState {
  return call.status === "answered" ? "output-available" : "no-result";
}

export const TERMINAL_STATES: ReadonlySet<ToolRowState> = new Set([
  "output-available",
  "output-error",
  "output-denied",
  "cancelled",
  "no-result",
]);

/** Payloads bigger than this render raw — pretty-printing a megabyte of JSON
 * buys nothing and costs a parse + reserialize on the render path. */
const PRETTY_PRINT_MAX_CHARS = 20_000;

/**
 * Display formatting for a projected tool payload (spec:
 * chat-tool-call-detail, "payloads render inert"): valid JSON under the
 * threshold re-indents for reading; anything else renders verbatim. Never
 * interpreted, never mutated at the source.
 */
export function formatPayload(text: string | null | undefined): string | null {
  if (text == null) return null;
  if (text.length > PRETTY_PRINT_MAX_CHARS) return text;
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

/** Head kept when a path is shortened, and the width one is shortened to. */
const STEP_PATH_HEAD_CHARS = 10;
const STEP_PATH_DISPLAY_MAX = 44;

/**
 * Shortens an over-long path for display, keeping its tail.
 *
 * The tail is the file name — the part a reader identifies a path by — so it is
 * the part that must survive. The full value stays reachable as the chip's
 * accessible name and title, because a shortening that changes which file the
 * chip names would be worse than no chip at all.
 */
export function shortenStepPath(path: string, max = STEP_PATH_DISPLAY_MAX): string {
  if (path.length <= max) return path;
  const tail = Math.max(1, max - STEP_PATH_HEAD_CHARS - 1);
  return `${path.slice(0, STEP_PATH_HEAD_CHARS)}…${path.slice(path.length - tail)}`;
}

/**
 * Turns the projection's step summary into what a row draws (spec:
 * chat-tool-trail).
 *
 * Shown-versus-total and display shortening only. The LABEL is passed through
 * untouched: it is composed Rust-side against the outcome it describes, and a
 * second composition here would drift from the first with only one of the two
 * checkable (spec: chat-tool-trail, "Naming does not guess").
 */
export function stepRow(step: ToolStepSummaryV1 | null | undefined): ToolStepRow | null {
  if (!step) return null;
  const paths = step.paths.map((entry) => ({
    path: entry.path,
    display: shortenStepPath(entry.path),
    sensitive: entry.is_sensitive,
  }));
  // Stated only when there is a visible list to qualify. With no chips at all
  // nothing is being presented as complete, and the label already carries the
  // count — a bare "0 of 3" would be an affordance standing in for the
  // difference, which the surface must not invent.
  const understated = paths.length > 0 && paths.length < step.path_total;
  return {
    label: step.label,
    paths,
    boundNote: understated ? `${paths.length} of ${step.path_total} paths` : null,
  };
}

export function formatElapsed(ms: number): string {
  if (ms < 1000) return "<1s";
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainderSeconds = seconds % 60;
  return remainderSeconds === 0 ? `${minutes}m` : `${minutes}m ${remainderSeconds}s`;
}
