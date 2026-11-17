import type { ChatToolCallView } from "../../lib/generated/contracts";

/**
 * Composing one standalone document out of a conversation
 * (spec: lean-report-export).
 *
 * Pure on purpose. Both destinations — the `.md` file the backend writes and
 * the print sheet the webview print engine writes as a PDF — read the SAME
 * selection out of this module, so a turn that is excluded from one cannot
 * survive into the other. The alternative, each surface filtering for itself,
 * is two chances to leak a turn the reader deselected.
 */

/** One transcript turn, as the export sees it. */
export type ExportTurn = {
  id: string;
  role: "user" | "assistant" | "system";
  text: string;
  toolCalls: readonly ChatToolCallView[];
};

/** What the reader ticked in the export dialog. */
export type ExportSelection = {
  /** Ids of the turns to include, in whatever order the caller holds them. */
  turnIds: ReadonlySet<string>;
  /**
   * Include each turn's tool round — the call names and their JSON arguments
   * and results.
   *
   * Off by default and never implied by anything else. The spec is explicit
   * that internal tool payloads leave only when a reader asks for them, and
   * those payloads are the part of a transcript most likely to carry a
   * connection string, a row of somebody's data, or a file path.
   */
  includeToolCalls: boolean;
  /**
   * Include the model's reasoning.
   *
   * Only a live run has any: reasoning is never persisted, so a reloaded
   * conversation carries none and this toggle has nothing to add to it. The
   * dialog says so rather than offering a switch that silently does nothing.
   */
  includeReasoning: boolean;
};

export type ExportMetadata = {
  title: string;
  /** Wall-clock of the export itself, injected so this module stays pure. */
  generatedAtMs: number;
  /** Reasoning captured from the live run, oldest first. Empty is normal. */
  reasoning?: readonly string[];
};

/** Turns that may ever appear in an export, before the reader chooses among them.
 *
 * A system turn is dropped here and cannot be selected back in: the spec
 * requires an export to omit raw system prompts, and a checkbox that could
 * re-admit one would make that requirement a preference. An empty turn goes
 * too — a heading over nothing is not a section, and a streaming turn that was
 * cancelled before its first token leaves exactly that.
 */
export function exportableTurns(turns: readonly ExportTurn[]): ExportTurn[] {
  return turns.filter((turn) => turn.role !== "system" && turn.text.trim() !== "");
}

/** The chosen turns, in transcript order. */
export function selectedTurns(turns: readonly ExportTurn[], selection: ExportSelection): ExportTurn[] {
  return exportableTurns(turns).filter((turn) => selection.turnIds.has(turn.id));
}

/**
 * What a call's status is called in a document.
 *
 * The wire status is `answered`/`unanswered`, which describes the projection's
 * bookkeeping rather than what happened. On a page read by someone who was not
 * watching the run, "unanswered" invites the reading "the tool refused"; it
 * means the transcript was captured before the result landed, so it says that.
 */
export const TOOL_STATUS_LABEL: Record<ChatToolCallView["status"], string> = {
  answered: "returned a result",
  unanswered: "no result recorded",
};

const ROLE_HEADING: Record<ExportTurn["role"], string> = {
  user: "Question",
  assistant: "Answer",
  // Unreachable — `exportableTurns` drops system turns — and spelled out anyway
  // so a future role cannot reach the document under a blank heading.
  system: "System",
};

/**
 * A GitHub-flavoured Markdown document for `turns`.
 *
 * Assistant prose is already Markdown, so it passes through untouched: the
 * tables, fences, and headings a model wrote are exactly what a `.md` file
 * should carry, and re-formatting them here could only degrade them. What this
 * adds is the frame — title, metadata, one heading per turn — and the
 * separators that keep two turns from reading as one.
 */
export function buildMarkdownDocument(
  turns: readonly ExportTurn[],
  selection: ExportSelection,
  metadata: ExportMetadata,
): string {
  const chosen = selectedTurns(turns, selection);
  const title = metadata.title.trim() === "" ? "Untitled conversation" : metadata.title.trim();
  const blocks: string[] = [`# ${title}`, metadataBlock(chosen.length, metadata.generatedAtMs)];

  const reasoning = selection.includeReasoning ? (metadata.reasoning ?? []) : [];
  if (reasoning.length > 0) {
    blocks.push("## Reasoning", ...reasoning.map((segment) => blockquote(segment)));
  }

  for (const turn of chosen) {
    blocks.push(`## ${ROLE_HEADING[turn.role]}`, turn.text.trim());
    if (selection.includeToolCalls && turn.toolCalls.length > 0) {
      blocks.push("### Tools used", ...turn.toolCalls.map(toolCallBlock));
    }
  }

  if (chosen.length === 0) {
    blocks.push("_No turns were selected for this export._");
  }

  // One trailing newline: a text file ends with one, and exactly one.
  return `${blocks.join("\n\n")}\n`;
}

/**
 * `YYYY-MM-DD HH:MM` in the reader's own zone.
 *
 * Deliberately not ISO-with-offset: this line is read by a person looking at a
 * document, not parsed. Exported without seconds for the same reason.
 */
export function formatExportTimestamp(atMs: number): string {
  const at = new Date(atMs);
  if (Number.isNaN(at.getTime())) return "unknown";
  const pad = (value: number) => String(value).padStart(2, "0");
  return (
    `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}` +
    ` ${pad(at.getHours())}:${pad(at.getMinutes())}`
  );
}

/**
 * A file name for the document, before the backend sanitizes it.
 *
 * Sanitized again on the Rust side, which is the boundary that matters — this
 * is only about the name the save dialog opens with, so the reader recognizes
 * their own conversation in it.
 */
export function suggestedFileName(title: string): string {
  const trimmed = title.trim();
  return trimmed === "" ? "conversation" : trimmed;
}

/** Two trailing spaces are Markdown's hard line break, so the two facts sit on
 * their own lines inside one paragraph instead of reflowing into a sentence. */
function metadataBlock(turnCount: number, generatedAtMs: number): string {
  const turns = turnCount === 1 ? "1 turn" : `${turnCount} turns`;
  const facts = [`**Exported** ${formatExportTimestamp(generatedAtMs)}`, `**Contents** ${turns}`].join("  \n");
  // The rule needs a genuinely blank line above it; a whitespace-only one would
  // read as a thematic break here and as a setext underline in a stricter
  // parser, which would silently turn the last fact into a heading.
  return `${facts}\n\n---`;
}

/** Markdown blockquote, with every line prefixed — a `>` on the first line
 * only stops quoting at the first blank line inside the segment. */
function blockquote(text: string): string {
  return text
    .trim()
    .split("\n")
    .map((line) => (line.trim() === "" ? ">" : `> ${line}`))
    .join("\n");
}

/**
 * One tool call as a named list plus fenced payloads.
 *
 * Fenced as `json` without checking that the payload parses: the arguments and
 * the result arrive as text the provider or server produced, and a fence tag is
 * a hint to a highlighter, not an assertion. Re-serializing to prove it is JSON
 * would rewrite a payload the reader asked to see verbatim, and truncation
 * markers are carried through for the same reason — a capped payload must not
 * read as a complete one.
 */
function toolCallBlock(call: ChatToolCallView): string {
  const lines = [`- **${call.name}** — ${TOOL_STATUS_LABEL[call.status]}`];
  if (call.arguments !== null && call.arguments !== undefined) {
    lines.push(fence(call.arguments, call.arguments_truncated, "Arguments"));
  }
  if (call.result !== null && call.result !== undefined) {
    lines.push(fence(call.result, call.result_truncated, "Result"));
  }
  return lines.join("\n\n");
}

function fence(payload: string, truncated: boolean, label: string): string {
  const heading = truncated ? `${label} (truncated)` : label;
  return [`${heading}:`, "```json", payload.trim(), "```"].join("\n");
}
