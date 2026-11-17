import { useEffect, useRef } from "react";
import ReactDOM from "react-dom";
import ReactMarkdown, { type Components } from "react-markdown";
import rehypeSanitize from "rehype-sanitize";
import remarkGfm from "remark-gfm";
import { documentExportPdf } from "../../lib/native";
import type { PluggableList } from "unified";
import { SANITIZE_SCHEMA } from "../chat/markdownPipeline";
import {
  formatExportTimestamp,
  selectedTurns,
  suggestedFileName,
  TOOL_STATUS_LABEL,
  type ExportMetadata,
  type ExportSelection,
  type ExportTurn,
} from "./exportDocument";

/** What came of asking the backend to write a PDF. */
export type PrintOutcome = "printed" | "unsupported" | "cancelled";

/**
 * How long to keep waiting for `beforeprint` after a non-Windows `window.print()`
 * returns without having fired it, before concluding the webview cannot print.
 *
 * Chromium fires the event synchronously and then blocks on its own preview.
 * This timeout exists for an engine that dispatches the event a task later,
 * and for the one that does nothing at all.
 */
const PRINT_DIALOG_GRACE_MS = 600;

/** Same sanitizer as chat; highlighting is omitted — print CSS forces ink black. */
const PRINT_REHYPE: PluggableList = [[rehypeSanitize, SANITIZE_SCHEMA]];
const PRINT_REMARK: PluggableList = [remarkGfm];

/** A printed page states link destinations and task state without carrying an
 * interactive anchor or checkbox (spec: lean-report-export). */
const PRINT_COMPONENTS: Components = {
  a({ href, children }) {
    return (
      <span>
        {children}
        {href ? <span className="xf-print-url"> ({href})</span> : null}
      </span>
    );
  },
  input({ type, checked }) {
    if (type !== "checkbox") return null;
    return <span className="xf-print-task-state">{checked ? "☒" : "☐"}</span>;
  },
};

export type PrintDocumentProps = {
  turns: readonly ExportTurn[];
  selection: ExportSelection;
  metadata: ExportMetadata;
  /** Called once, after the save has been asked for and either written or declined. */
  onDone: (outcome: PrintOutcome, fileName?: string | null) => void;
};

/**
 * The printable document, mounted on `<body>` beside the app root.
 *
 * A separate tree rather than print styles over the transcript: `MessageList`
 * is virtualized, so most of what a reader selected is not in the DOM at all
 * and could not be printed from there. Printing what is on screen would
 * silently produce a document containing whichever turns happened to be
 * scrolled into view.
 *
 * Same sanitizer as the chat surface, so untrusted model output is treated
 * the same on paper. Highlighting is skipped: print CSS already forces every
 * token to ink black, and running the grammar set on a large transcript is
 * work the reader cannot see.
 *
 * PDF is written by the backend (`document_export_pdf`) through the native
 * save dialog. `window.print()` is not used on Windows: WebView2's print
 * preview blocks the renderer until it is dismissed, and that preview can
 * fail to appear, leaving Close and Cancel as no-ops for as long as the
 * process lives.
 */
export function PrintDocument({ turns, selection, metadata, onDone }: PrintDocumentProps) {
  const chosen = selectedTurns(turns, selection);
  const reasoning = selection.includeReasoning ? (metadata.reasoning ?? []) : [];
  // The parent re-renders on every streamed token and rebuilds `onDone`. The
  // offer must not live in that callback's identity: an effect that cleaned
  // up on each new function cancelled the save dialog before it opened, while
  // Close — a plain click — kept working.
  const onDoneRef = useRef(onDone);
  onDoneRef.current = onDone;

  useEffect(() => {
    let cancelled = false;
    let layoutFrame = 0;
    let printFrame = 0;
    const suggested = suggestedFileName(metadata.title);

    // A print-only code block can be the first consumer of the bundled mono
    // face. Wait until every requested face has either loaded or failed, then
    // give the engine two frames: one to commit this tree and one to lay it out
    // under print media. Starting earlier can capture fallback glyph metrics or
    // a blank first page.
    const fontsReady = document.fonts?.ready ?? Promise.resolve();
    void fontsReady.then(() => {
      if (cancelled) return;
      layoutFrame = requestAnimationFrame(() => {
        printFrame = requestAnimationFrame(() => {
          if (cancelled) return;
          void offerPdf(suggested).then((result) => {
            if (!cancelled) onDoneRef.current(result.outcome, result.fileName);
          });
        });
      });
    });

    return () => {
      cancelled = true;
      cancelAnimationFrame(layoutFrame);
      cancelAnimationFrame(printFrame);
    };
    // One offer per print session (`generatedAtMs` is frozen at click). Title
    // is read once above so a later rename cannot cancel the dialog.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- print session
  }, [metadata.generatedAtMs]);

  const title = metadata.title.trim() === "" ? "Untitled conversation" : metadata.title.trim();

  return ReactDOM.createPortal(
    // `aria-hidden`: on screen this tree is `display: none`, and a hidden
    // duplicate of the whole transcript is exactly what a screen reader must
    // not walk through.
    <article aria-hidden className="xf-print-document">
      <header className="xf-print-header">
        <h1>{title}</h1>
        <p>
          Exported {formatExportTimestamp(metadata.generatedAtMs)} ·{" "}
          {chosen.length === 1 ? "1 turn" : `${chosen.length} turns`}
        </p>
      </header>

      {reasoning.length > 0 ? (
        <section className="xf-print-section">
          <h2>Reasoning</h2>
          {reasoning.map((segment, index) => (
            <blockquote className="xf-print-reasoning" key={index}>
              {segment}
            </blockquote>
          ))}
        </section>
      ) : null}

      {chosen.length === 0 ? <p className="xf-print-empty">No turns were selected for this export.</p> : null}

      {chosen.map((turn) => (
        <section className="xf-print-section" key={turn.id}>
          <h2>{turn.role === "user" ? "Question" : "Answer"}</h2>
          <ReactMarkdown components={PRINT_COMPONENTS} rehypePlugins={PRINT_REHYPE} remarkPlugins={PRINT_REMARK}>
            {turn.text}
          </ReactMarkdown>
          {selection.includeToolCalls && turn.toolCalls.length > 0 ? (
            <div className="xf-print-tools">
              <h3>Tools used</h3>
              {turn.toolCalls.map((call) => (
                <div className="xf-print-tool" key={call.call_id}>
                  <p>
                    <strong>{call.name}</strong> — {TOOL_STATUS_LABEL[call.status]}
                  </p>
                  {call.arguments != null ? (
                    <pre>
                      {call.arguments_truncated ? "Arguments (truncated):\n" : "Arguments:\n"}
                      {call.arguments}
                    </pre>
                  ) : null}
                  {call.result != null ? (
                    <pre>
                      {call.result_truncated ? "Result (truncated):\n" : "Result:\n"}
                      {call.result}
                    </pre>
                  ) : null}
                </div>
              ))}
            </div>
          ) : null}
        </section>
      ))}
    </article>,
    document.body,
  );
}

type OfferResult = { outcome: PrintOutcome; fileName?: string | null };

/**
 * Asks the backend to write a PDF of the mounted sheet.
 *
 * A dismissed save dialog is `cancelled`. A platform that cannot produce a
 * PDF without `window.print()` says so with a stable error, and only then
 * does this fall back to the print dialog — never on Windows, where that
 * call blocks the renderer. The toast lives on the dialog so a Close that
 * unmounted this tree cannot announce a file after the reader left.
 */
async function offerPdf(suggestedName: string): Promise<OfferResult> {
  try {
    const result = await documentExportPdf(suggestedName);
    if (!result.saved) return { outcome: "cancelled" };
    return { outcome: "printed", fileName: result.file_name };
  } catch (cause) {
    if (String(cause).includes("PRINT_VIA_DIALOG")) {
      return { outcome: await askToPrint() };
    }
    return { outcome: "unsupported" };
  }
}

/**
 * Asks the platform to print, and reports whether it actually offered to.
 *
 * Used only on engines where `document_export_pdf` cannot write a PDF itself
 * (not Windows WebView2). `window.print` is a function on every platform this
 * app ships to, including the ones where calling it does nothing, so its
 * presence proves nothing. What distinguishes them is `beforeprint`.
 */
async function askToPrint(): Promise<PrintOutcome> {
  let offered = false;
  const observe = () => {
    offered = true;
  };
  window.addEventListener("beforeprint", observe);
  try {
    window.print();
    if (offered) return "printed";
    await new Promise((resolve) => {
      window.setTimeout(resolve, PRINT_DIALOG_GRACE_MS);
    });
    return offered ? "printed" : "unsupported";
  } finally {
    window.removeEventListener("beforeprint", observe);
  }
}
