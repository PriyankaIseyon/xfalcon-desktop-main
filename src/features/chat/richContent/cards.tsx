import { useContext, useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { ChartNoAxesColumn, Check, Copy, Loader2, Maximize2, RotateCcw } from "lucide-react";
import { ARTIFACT_ACTION_CLASS } from "./artifactAction";
import { ArtifactHostActions } from "./artifactActionsContext";
import { cn } from "../../../lib/cn";
import { previewDocumentRegister, previewDocumentSrc, type PreviewDocumentKind } from "../../../lib/native";
import { writeClipboardText } from "../clipboard";
import { HTML_PREVIEW_MAX_BYTES, resolveHtmlTitle, type TableData } from "./detect";
import { openCanvasArtifact } from "../../canvas/canvasStore";
import { Skeleton } from "../../../ui/Skeleton";
import { formatNumber } from "../../../lib/formatNumber";
import { chartSpecFor, numberFormatsFor, PROJECT_TABLE_CHART_LIMIT_MESSAGE } from "./tableChart";

/** Rows shown before the table folds into a remainder footer
 * (spec: chat-rich-content — bounded DOM, full source still copyable). */
export const TABLE_ROW_CAP = 100;

type PreviewState =
  | { status: "registering"; kind: PreviewDocumentKind; content: string }
  | { status: "ready"; kind: PreviewDocumentKind; content: string; documentId: string; src: string }
  | { status: "failed"; kind: PreviewDocumentKind; content: string };

/** Registers `content` with the preview store and resolves the frame URL.
 * Content-addressed on the Rust side, so identity re-renders are free;
 * a registration failure degrades to the source view, never a blank card. */
function useRegisteredPreview(
  kind: PreviewDocumentKind,
  content: string,
  disabled: boolean,
  register?: () => Promise<string>,
): PreviewState {
  const [state, setState] = useState<PreviewState>({ status: "registering", kind, content });
  // Read at registration time, not a dependency: the document IS `content`,
  // and a caller's inline function would otherwise re-register on every render.
  const registerRef = useRef(register);
  registerRef.current = register;
  useEffect(() => {
    if (disabled) return undefined;
    let live = true;
    setState({ status: "registering", kind, content });
    (registerRef.current ?? (() => previewDocumentRegister(kind, content)))().then(
      (id) => {
        // The id is kept, not just the URL it builds: reading it back out of
        // `src` with a pattern would make the store's id format a thing two
        // modules have to agree about, and the agreement would break silently.
        if (live) setState({ status: "ready", kind, content, documentId: id, src: previewDocumentSrc(id) });
      },
      () => {
        if (live) setState({ status: "failed", kind, content });
      },
    );
    return () => {
      live = false;
    };
  }, [kind, content, disabled]);
  // Effects run after paint. Do not expose a ready result from the previous
  // document during the render that starts a new registration.
  if (disabled || state.kind !== kind || state.content !== content) {
    return { status: "registering", kind, content };
  }
  return state;
}

/** Copy, named for the artifact it acts on. An answer routinely carries
 * several frames, and a row of identically named "Copy" controls leaves a
 * screen-reader user unable to tell the report's from the table's. */
function CopyAction({ text, artifact }: { text: string; artifact: string }) {
  type CopyState = "idle" | "copying" | "copied" | "failed";
  const [copyState, setCopyState] = useState<{ source: string; value: CopyState }>({ source: text, value: "idle" });
  const operation = useRef(0);
  const resetTimer = useRef<number | null>(null);
  const currentText = useRef(text);
  currentText.current = text;
  const state = copyState.source === text ? copyState.value : "idle";

  useEffect(() => {
    return () => {
      operation.current += 1;
      if (resetTimer.current !== null) window.clearTimeout(resetTimer.current);
    };
  }, []);

  const copy = async () => {
    if (state === "copying") return;
    const currentOperation = ++operation.current;
    setCopyState({ source: text, value: "copying" });
    try {
      await writeClipboardText(text);
      if (operation.current !== currentOperation || currentText.current !== text) return;
      setCopyState({ source: text, value: "copied" });
      resetTimer.current = window.setTimeout(() => {
        if (operation.current === currentOperation && currentText.current === text) {
          setCopyState({ source: text, value: "idle" });
        }
      }, 2000);
    } catch {
      if (operation.current === currentOperation && currentText.current === text) {
        setCopyState({ source: text, value: "failed" });
      }
    }
  };

  const label =
    state === "copying"
      ? `Copying ${artifact}`
      : state === "copied"
        ? `${artifact} copied`
        : state === "failed"
          ? `Try to copy ${artifact} again`
          : `Copy ${artifact} to clipboard`;
  return (
    <button
      aria-busy={state === "copying" || undefined}
      aria-label={label}
      className={ARTIFACT_ACTION_CLASS}
      data-state={state}
      disabled={state === "copying"}
      type="button"
      onClick={() => {
        void copy();
      }}
    >
      {state === "copying" ? (
        <Loader2 aria-hidden className="animate-spin" size={14} />
      ) : state === "copied" ? (
        <Check aria-hidden className="badge-pop text-success" size={14} />
      ) : state === "failed" ? (
        <RotateCcw aria-hidden size={14} />
      ) : (
        <Copy aria-hidden size={14} />
      )}
      <span aria-live="polite">
        {state === "copying"
          ? "Copying…"
          : state === "copied"
            ? "Copied"
            : state === "failed"
              ? "Try copy again"
              : "Copy"}
      </span>
    </button>
  );
}

function SourceAction({
  open,
  artifact,
  controls,
  onToggle,
}: {
  open: boolean;
  artifact: string;
  controls: string;
  onToggle: () => void;
}) {
  return (
    <button
      aria-controls={controls}
      aria-expanded={open}
      aria-label={`${open ? "Hide" : "Show"} source of ${artifact}`}
      className={ARTIFACT_ACTION_CLASS}
      type="button"
      onClick={onToggle}
    >
      {open ? "Hide source" : "Show source"}
    </button>
  );
}

/**
 * The one frame every fenced artifact renders in (spec: chat-rich-content).
 *
 * It owns all of the chrome — the identity band, the action row, and the
 * building state — and each type contributes only a body. That is the whole
 * point: the chrome used to be built four times by hand and had drifted four
 * ways, with one action carrying three different accessible names and two
 * rungs unable to give their source back. There is one of it now, so it
 * cannot drift.
 *
 * Action availability is computed HERE from the artifact's state, never
 * passed in per type, and the list is the seam a later action lands on
 * (the project save action is supplied to it) rather than a fifth
 * hand-authored header.
 */
export function ArtifactFrame({
  title,
  description,
  source,
  sourceView,
  saveAction,
  canvasAction,
  degraded = false,
  building = false,
  footer,
  children,
}: {
  /** What the artifact IS — a document's own title, a table's caption, the
   * fence's language. Model-authored, so it renders inert and clamped. */
  title: string;
  /** What it MEASURES. Omitted when nothing factual can be derived: an empty
   * slot is honest, and filler trains the reader to skip the slot that
   * sometimes carries the row count. */
  description?: string;
  /** The artifact's complete source. Absent while it is still arriving —
   * copying half a document yields something that does not work. */
  source?: string;
  /** The same string `source` carries, as a readable tree. Absent when the
   * body already IS the source, which is why the code artifact carries no
   * toggle. */
  sourceView?: ReactNode;
  /**
   * "Save to project", supplied by the one artifact type that can be saved.
   *
   * A slot rather than a computed action, and the split is deliberate: WHEN the
   * action may appear is decided here (never while building, alongside the
   * other actions, in the same row), while WHAT it is stays with the type that
   * holds the document's identity. Computing it here would mean this frame —
   * shared by four artifact types — importing the reports feature.
   */
  saveAction?: ReactNode;
  /** "Open in panel", for interactive inspection beside the chat. */
  canvasAction?: ReactNode;
  /** The rendered body could not be produced — registration failed, or the
   * document is over the preview cap. The source shows with no toggle back
   * to a body that does not exist. */
  degraded?: boolean;
  building?: boolean;
  /** A note that belongs under the body: a table's remainder, the reason a
   * preview degraded. */
  footer?: ReactNode;
  children?: ReactNode;
}) {
  const [showSource, setShowSource] = useState(false);
  const hostActions = useContext(ArtifactHostActions);
  const titleId = useId();
  const sourceId = useId();
  const hasBody = children !== undefined && !degraded;
  const sourceOpen = degraded || showSource;
  const hasActions =
    !building &&
    (source !== undefined ||
      (sourceView !== undefined && hasBody) ||
      (canvasAction !== undefined && canvasAction !== null) ||
      (saveAction !== undefined && saveAction !== null) ||
      (hostActions !== undefined && hostActions !== null));

  return (
    // The status role exists only while building. A settled artifact is not a
    // status, and leaving the role in place would re-announce the whole
    // artifact on any later change inside it.
    // Cleared for everything inside: a host's actions are for this card, not
    // for a card nested in its body.
    <ArtifactHostActions.Provider value={null}>
      <section
        aria-labelledby={titleId}
        className={cn("rich-card", degraded && "rich-card--degraded")}
        role={building ? "status" : undefined}
      >
        <div className="rich-card__header">
          <span className="rich-card__identity">
            <span className="rich-card__title" id={titleId}>
              {title}
            </span>
            {description ? <span className="rich-card__meta">{description}</span> : null}
          </span>
          {hasActions ? (
            <span aria-label={`Actions for ${title}`} className="rich-card__actions" role="group">
              {source !== undefined ? <CopyAction artifact={title} text={source} /> : null}
              {sourceView !== undefined && hasBody ? (
                <SourceAction
                  artifact={title}
                  controls={sourceId}
                  open={sourceOpen}
                  onToggle={() => setShowSource((previous) => !previous)}
                />
              ) : null}
              {canvasAction}
              {/* Last in the row, and never while building: an identity the
                backend cannot resolve yet would produce a save that can only
                answer "not found". */}
              {saveAction}
              {hostActions}
            </span>
          ) : null}
        </div>
        {/* Hidden, never unmounted: re-mounting a preview re-registers the
          document with the store and restarts whatever its scripts were
          doing, so the toggle would cost a reload. */}
        {hasBody ? <div className={cn(sourceOpen && "hidden")}>{children}</div> : null}
        {footer}
        {sourceView !== undefined ? (
          <pre
            aria-label={`${title} source`}
            className={cn("rich-card__code", !sourceOpen && "hidden")}
            id={sourceId}
            tabIndex={sourceOpen ? 0 : -1}
          >
            {sourceView}
          </pre>
        ) : null}
      </section>
    </ArtifactHostActions.Provider>
  );
}

/** The body is the source, so there is nothing to toggle to. `language` is the
 * RESOLVED display name from `codeLanguages.ts` — "Snowflake SQL", not the
 * `snowflake` tag the model happened to type — so the header names the language
 * rather than repeating the model's shorthand. */
export function CodeArtifact({ language, code, children }: { language: string; code: string; children: ReactNode }) {
  return (
    <ArtifactFrame source={code} title={language}>
      <pre aria-label={`${language} source`} className="rich-card__code" tabIndex={0}>
        {children}
      </pre>
    </ArtifactFrame>
  );
}

/** `4,182 rows · 7 columns` — the artifact's TRUE totals, not what the capped
 * body happens to render, so the description, the body, and the remainder
 * footer agree arithmetically about the same table. */
function describeTable(table: TableData): string {
  const rows = `${formatNumber(table.rows.length)} row${table.rows.length === 1 ? "" : "s"}`;
  const columns = `${formatNumber(table.headers.length)} column${table.headers.length === 1 ? "" : "s"}`;
  return `${rows} · ${columns}`;
}

/**
 * A canvas tab identity for one table.
 *
 * Title and headers alone are not enough: an answer routinely renders the same
 * query for two periods, which produces two tables with identical titles and
 * identical headers. Sharing an id would make the second silently replace the
 * first, so the shape and the first row join the key -- enough to separate two
 * real tables, and still stable across the re-renders of one.
 */
function tableArtifactId(title: string, table: TableData): string {
  const shape = `${table.headers.length}x${table.rows.length}`;
  const firstRow = (table.rows[0] ?? []).join("\u0001");
  return `table:${title}:${table.headers.join("\u0001")}:${shape}:${firstRow}`;
}

/** Capped data table — cells are TEXT NODES only (spec: chat-rich-content;
 * upload-cell-injection doctrine: display-time neutralization, no markup
 * interpretation, stored content untouched). */
export function TableArtifact({
  table,
  source,
  sourceView,
  title,
  building = false,
  allowRowFallback = false,
}: {
  /** Absent while building: the fence has not closed, so nothing has parsed
   * and no count is known. */
  table?: TableData;
  source?: string;
  sourceView?: ReactNode;
  title: string;
  building?: boolean;
  /** Project CSV/TSV files can chart row counts when they have no measures. */
  allowRowFallback?: boolean;
}) {
  if (building || !table) {
    return (
      <ArtifactFrame building description="Building…" title={title}>
        <div className="rich-card__table-ghost">
          <Skeleton />
          <Skeleton />
          <Skeleton />
        </div>
      </ArtifactFrame>
    );
  }
  const visible = table.rows.slice(0, TABLE_ROW_CAP);
  const remainder = table.rows.length - visible.length;
  // Resolved over the WHOLE table, not the capped body: a row past the cap
  // that made a column textual must still exclude it from numeric series and sort.
  const formats = numberFormatsFor(table);
  const chartable = chartSpecFor(table, formats, allowRowFallback);
  const chartTooLarge = allowRowFallback && table.rows.length > 0 && table.headers.length > 0 && chartable === null;
  return (
    <ArtifactFrame
      canvasAction={
        <>
          <button
            aria-label={`Open ${title} in the side panel`}
            className={ARTIFACT_ACTION_CLASS}
            type="button"
            onClick={() => {
              openCanvasArtifact({
                id: tableArtifactId(title, table),
                type: "table",
                title,
                columns: table.headers,
                rows: table.rows,
                // The format the card already resolved is the one the grid needs
                // to sort numerically -- a chat table's cells are all strings, so
                // a grid left to infer would put "100" before "50".
                numberFormats: formats,
              });
            }}
          >
            <Maximize2 aria-hidden size={14} strokeWidth={1.8} />
            Open in panel
          </button>
          {chartable !== null ? (
            <button
              aria-label={`Chart ${title} in the side panel`}
              className={ARTIFACT_ACTION_CLASS}
              type="button"
              onClick={() => {
                openCanvasArtifact({
                  id: `chart:${tableArtifactId(title, table)}`,
                  type: "chart",
                  title,
                  chartType: "bar",
                  data: chartable.data,
                  xKey: chartable.xKey,
                  yKeys: chartable.yKeys,
                  seriesLabels: chartable.seriesLabels,
                });
              }}
            >
              <ChartNoAxesColumn aria-hidden size={14} strokeWidth={1.8} />
              Chart in panel
            </button>
          ) : null}
        </>
      }
      description={describeTable(table)}
      footer={
        <>
          {chartTooLarge ? (
            <p className="rich-card__footer type-caption text-muted">{PROJECT_TABLE_CHART_LIMIT_MESSAGE}</p>
          ) : null}
          {remainder > 0 ? (
            <p className="rich-card__footer type-caption text-muted">
              {formatNumber(remainder)} more row{remainder === 1 ? "" : "s"} — copy the source for the full set
            </p>
          ) : null}
        </>
      }
      source={source}
      sourceView={sourceView}
      title={title}
    >
      <div aria-label={`${title} table`} className="rich-card__scroll" role="region" tabIndex={0}>
        <table className="rich-table">
          <thead>
            <tr>
              {table.headers.map((header, index) => (
                <th key={`${index}-${header}`}>{header}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {visible.map((row, rowIndex) => (
              <tr key={rowIndex}>
                {row.map((cell, cellIndex) => (
                  <td key={cellIndex}>{cell}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </ArtifactFrame>
  );
}

/**
 * Live document preview (spec: chat-rich-content) — the ONE preview path,
 * shared by the html and svg bodies. Isolation contract: the document loads
 * from the `xf-preview` scheme — its own non-app origin with a per-response
 * CSP that allows the document's inline styles/scripts while denying ALL
 * network (spec: preview-protocol). Local-scheme URLs (`data:`/`blob:`/srcdoc)
 * are banned here: WebView2 makes them inherit the app CSP, which refuses the
 * document's own styling — the Times-on-white report defect.
 * `sandbox="allow-scripts"` and NEVER `allow-same-origin` stays as defense in
 * depth: even on its own origin the document gets no DOM, storage, or Tauri
 * IPC reach into the app.
 */
function PreviewArtifact({
  kind,
  content,
  title,
  typeName,
  building,
  saveAction,
  register,
  maxBytes = HTML_PREVIEW_MAX_BYTES,
  allowPanel = true,
}: {
  kind: PreviewDocumentKind;
  /** The document span — the same string Copy yields and the source view
   * shows. The raw fence body is NOT used: anything trailing `</html>` would
   * make the three disagree about one artifact. */
  content: string;
  title: string;
  /** What the artifact is, for the description slot once it has settled.
   * Absent when the title already says it — a document with no `<title>` is
   * called "HTML document", and repeating that below itself is filler. */
  typeName?: string;
  building: boolean;
  saveAction?: ReactNode;
  /** How the document reaches the preview store, when not from the renderer:
   * a project file registers from disk, by path. */
  register?: () => Promise<string>;
  /** The largest document this card previews; past it the card shows source. */
  maxBytes?: number;
  /** Whether to offer the side panel. The panel re-registers a document from
   * its content if the store evicted it, which a renderer-supplied document
   * past the chat's bound could not do. */
  allowPanel?: boolean;
}) {
  const [loadedSrc, setLoadedSrc] = useState<string | null>(null);
  // Encoding a growing document on every token would restore the quadratic
  // streaming cost the block splitter removed. The cap matters only once the
  // fence is complete and registration can start.
  const byteLength = useMemo(() => (building ? 0 : new TextEncoder().encode(content).byteLength), [building, content]);
  const tooLarge = byteLength > maxBytes;
  // A partial document must never reach the store: it would register bytes
  // that are not the artifact and then be replaced on the next token.
  const preview = useRegisteredPreview(kind, content, tooLarge || building, register);
  const degraded = !building && (tooLarge || preview.status === "failed");

  const body = building ? (
    <Skeleton className="rich-card__frame-ghost" />
  ) : degraded ? undefined : preview.status === "ready" ? (
    <iframe
      className={cn("rich-card__frame", loadedSrc === preview.src && "rich-card__frame--loaded")}
      sandbox="allow-scripts"
      src={preview.src}
      title={title}
      onLoad={() => setLoadedSrc(preview.src)}
    />
  ) : (
    // Reserves the settled frame's exact height, so the ghost→frame swap
    // never moves the transcript.
    <Skeleton className="rich-card__frame-ghost" />
  );

  return (
    <ArtifactFrame
      building={building}
      degraded={degraded}
      description={building ? "Building…" : degraded ? "Source only" : typeName}
      footer={
        degraded ? (
          <p className="rich-card__footer type-caption text-muted" role="status">
            {tooLarge ? "Document too large to preview — showing source." : "Preview unavailable — showing source."}
          </p>
        ) : null
      }
      source={building ? undefined : content}
      saveAction={saveAction}
      canvasAction={
        !building && allowPanel && preview.status === "ready" ? (
          <button
            aria-label={`Open ${title} in the side panel`}
            className={ARTIFACT_ACTION_CLASS}
            type="button"
            onClick={() => {
              openCanvasArtifact({
                // Keyed by the id the store minted, which is content-addressed
                // -- so re-opening the same document reuses its tab, and two
                // documents that differ by a byte get two.
                id: `${kind}:${preview.documentId}`,
                type: "html",
                title,
                kind,
                documentId: preview.documentId,
                // The source travels with the tab: the preview store evicts
                // least-recently-served, so the id alone would come back as a
                // silently blank frame after another document pushed it out.
                content,
              });
            }}
          >
            <Maximize2 aria-hidden size={14} strokeWidth={1.8} />
            Open in panel
          </button>
        ) : undefined
      }
      // Inert text rather than the highlighted fence body: the highlighted
      // tree is the whole fence, and Copy, the body, and this view have to be
      // the same string (spec: chat-rich-content, "Copy and source agree").
      sourceView={building ? undefined : <code>{content}</code>}
      title={title}
    >
      {body}
    </ArtifactFrame>
  );
}

export function HtmlArtifact({
  html,
  building = false,
  saveAction,
  register,
  maxBytes,
  allowPanel,
}: {
  html: string;
  building?: boolean;
  register?: () => Promise<string>;
  maxBytes?: number;
  allowPanel?: boolean;
  /** "Save to project", when the document's source identity is one the backend
   * can resolve. Absent for a document still arriving, or one whose message has
   * not landed in the durable conversation yet (spec: reports). */
  saveAction?: ReactNode;
}) {
  // Resolved from whatever has arrived, with the same resolver the settled
  // state uses — a `<title>` lands among a document's first bytes, so the
  // settle changes the body and never the name.
  const named = resolveHtmlTitle(html);
  return (
    <PreviewArtifact
      allowPanel={allowPanel}
      building={building}
      content={html}
      kind="html"
      maxBytes={maxBytes}
      register={register}
      saveAction={saveAction}
      title={named ?? "HTML document"}
      typeName={named ? "HTML document" : undefined}
    />
  );
}

/** SVG preview — image semantics on the same isolation contract (the
 * protocol serves it as `image/svg+xml` with scripts refused). */
export function SvgArtifact({
  svg,
  building = false,
  register,
  maxBytes,
  allowPanel,
}: {
  svg: string;
  building?: boolean;
  register?: () => Promise<string>;
  maxBytes?: number;
  allowPanel?: boolean;
}) {
  return (
    <PreviewArtifact
      allowPanel={allowPanel}
      building={building}
      content={svg}
      kind="svg"
      maxBytes={maxBytes}
      register={register}
      title="SVG image"
    />
  );
}

/** The frame around the nested-prose pipeline, so a markdown fence's source
 * is reachable for the first time — it was the one rung that rendered bare,
 * which is why the markdown a reader was shown could not be copied back. No
 * building state: partial prose is readable. */
export function MarkdownArtifact({
  source,
  sourceView,
  children,
}: {
  source: string;
  sourceView: ReactNode;
  children: ReactNode;
}) {
  return (
    <ArtifactFrame source={source} sourceView={sourceView} title="Markdown">
      <div className="rich-card__prose">{children}</div>
    </ArtifactFrame>
  );
}
