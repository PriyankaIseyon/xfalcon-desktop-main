import React, { useMemo, useState, type ReactNode } from "react";
import { measureElement, useVirtualizer, type Virtualizer } from "@tanstack/react-virtual";
import { Columns2, Rows3 } from "lucide-react";
import { cn } from "../../lib/cn";
import { observeScrollRect } from "../../lib/virtualScrollRect";
import { Alert } from "../../ui/Alert";
import { SegmentedControl } from "../../ui/SegmentedControl";
import {
  parseUnifiedDiff,
  toSplitRows,
  type DiffRow,
  type DiffSection,
  type SplitDiffRow,
  type TextDiffSection,
} from "./diff";

export type DiffMode = "unified" | "split";

export interface FileDiffViewerProps {
  source: string;
  className?: string;
  /** The layout it opens in. The reader can still switch. */
  defaultMode?: DiffMode;
}

type RenderItem =
  | { kind: "header"; key: string; section: TextDiffSection }
  | { kind: "unified"; key: string; path: string | null; row: DiffRow }
  | { kind: "split"; key: string; path: string | null; row: SplitDiffRow }
  | { kind: "opaque"; key: string; content: string };

const VIRTUALIZE_AFTER_ROWS = 500;

const rowTone: Record<DiffRow["kind"], string> = {
  hunk: "bg-info-bg text-info",
  context: "text-secondary",
  add: "bg-success-bg text-foreground",
  delete: "bg-danger-bg text-foreground",
  note: "bg-warn-bg text-warn",
};

const changeLabel: Record<TextDiffSection["change"], string> = {
  created: "Created",
  deleted: "Deleted",
  changed: "Changed",
};

function rowLabel(row: DiffRow): string {
  if (row.kind === "add") return `Added line ${row.newLine}`;
  if (row.kind === "delete") return `Deleted line ${row.oldLine}`;
  if (row.kind === "context") return `Unchanged line ${row.oldLine}`;
  if (row.kind === "hunk") return "Diff hunk";
  return row.text.includes("truncated") ? "Diff preview truncated" : "Diff note";
}

const HIGHLIGHT_TOKEN = /("(?:\\.|[^"\\])*"|-?\d+(?:\.\d+)?|true|false|null|,)/g;
const HIGHLIGHT_LITERAL = /^-?\d|^(?:true|false|null)$/;

function highlighted(text: string, path: string | null): ReactNode {
  const extension = path?.split(".").pop()?.toLowerCase();
  if (extension !== "json" && extension !== "csv") return text;
  const tokens = text.split(HIGHLIGHT_TOKEN);
  return tokens.map((token, index) => {
    const className = token.startsWith('"')
      ? "text-info"
      : HIGHLIGHT_LITERAL.test(token)
        ? "text-warn"
        : token === ","
          ? "text-muted"
          : undefined;
    return (
      <span className={className} key={`${index}:${token}`}>
        {token}
      </span>
    );
  });
}

function UnifiedRow({ row, path }: { row: DiffRow; path: string | null }) {
  return (
    <div
      aria-label={rowLabel(row)}
      className={cn(
        "grid h-6 w-full min-w-max grid-cols-[3rem_3rem_1.5rem_1fr] font-mono text-xs leading-6",
        rowTone[row.kind],
      )}
    >
      <span aria-hidden className="select-none px-2 text-right text-muted">
        {row.oldLine ?? ""}
      </span>
      <span aria-hidden className="select-none border-r border-default px-2 text-right text-muted">
        {row.newLine ?? ""}
      </span>
      <span aria-hidden className="select-none text-center font-semibold">
        {row.marker}
      </span>
      <code className="whitespace-pre pr-4">{highlighted(row.text, path)}</code>
    </div>
  );
}

/** One half of a split row. A line wraps inside its own half rather than
 * scrolling: each half is only half the width, so an unwrapped long line would
 * either paint over the other half or be cut off with nothing to scroll it by.
 * Both halves share their row's height, so the two sides stay aligned. */
function SplitCell({ row, side, path }: { row: DiffRow | null; side: "old" | "new"; path: string | null }) {
  const line = side === "old" ? row?.oldLine : row?.newLine;
  return (
    <div
      className={cn(
        "grid min-h-6 min-w-0 grid-cols-[3rem_1.5rem_minmax(0,1fr)]",
        side === "old" && "border-r border-default",
        row ? rowTone[row.kind] : "bg-surface",
      )}
    >
      <span aria-hidden className="px-2 text-right text-muted">
        {line ?? ""}
      </span>
      <span aria-hidden className="text-center font-semibold">
        {row?.marker ?? ""}
      </span>
      <code className="whitespace-pre-wrap break-words pr-4">{row ? highlighted(row.text, path) : ""}</code>
    </div>
  );
}

function splitRowLabel(row: Extract<SplitDiffRow, { kind: "pair" }>): string {
  if (row.left?.kind === "context") return rowLabel(row.left);
  if (row.left && row.right) return `${rowLabel(row.left)}; ${rowLabel(row.right)}`;
  if (row.left) return rowLabel(row.left);
  if (row.right) return rowLabel(row.right);
  return "Empty diff row";
}

function SplitRow({ row, path }: { row: SplitDiffRow; path: string | null }) {
  if (row.kind === "meta") {
    return (
      <div className="min-w-[48rem]">
        <UnifiedRow path={path} row={row.row} />
      </div>
    );
  }
  return (
    <div aria-label={splitRowLabel(row)} className="grid min-h-6 min-w-[48rem] grid-cols-2 font-mono text-xs leading-6">
      <SplitCell path={path} row={row.left} side="old" />
      <SplitCell path={path} row={row.right} side="new" />
    </div>
  );
}

function SectionHeader({ section }: { section: TextDiffSection }) {
  return (
    // Sticky in the flow (non-virtualized) path: this file's own name stays
    // visible while its hunks scroll underneath, until the next file's
    // header reaches the top and displaces it. A virtualized long diff still
    // scrolls its header away with the rest of that row -- each row's own
    // absolutely-positioned wrapper is too short to give `sticky` a
    // containing block worth sticking within.
    <div className="sticky top-0 z-10 flex h-9 min-w-max items-center gap-2 border-y border-hairline bg-surface px-3">
      <span className="type-micro font-semibold uppercase tracking-wide text-muted">{changeLabel[section.change]}</span>
      <span className="font-mono text-xs text-secondary">{section.path ?? "Unnamed file"}</span>
    </div>
  );
}

function opaqueTitle(content: string): string {
  if (/binary/i.test(content)) return "Binary change";
  if (/renamed:/i.test(content)) return "Rename";
  if (/deleted:/i.test(content)) return "Delete";
  if (content.includes("diff truncated")) return "Preview limit";
  return "File operation";
}

function OpaqueSection({ content }: { content: string }) {
  return (
    <div className="border-y border-hairline bg-warn-bg px-3 py-2 text-warn">
      <p className="mb-1 text-xs font-semibold">{opaqueTitle(content)}</p>
      <pre className="whitespace-pre-wrap break-words font-mono text-xs leading-5">{content}</pre>
    </div>
  );
}

function renderItem(item: RenderItem): ReactNode {
  if (item.kind === "header") return <SectionHeader section={item.section} />;
  if (item.kind === "unified") return <UnifiedRow path={item.path} row={item.row} />;
  if (item.kind === "split") return <SplitRow path={item.path} row={item.row} />;
  return <OpaqueSection content={item.content} />;
}

function itemEstimate(item: RenderItem): number {
  if (item.kind === "header") return 36;
  if (item.kind === "opaque") return 64;
  return 24;
}

/**
 * A row that measures 0 (no layout engine, e.g. jsdom) or momentarily
 * shrinks below its own last measurement would otherwise feed the
 * virtualizer a smaller size than before on every remeasure -- each
 * `measureElement` ref attach schedules the resulting resize as a state
 * update, which schedules another remeasure, without ever converging. This
 * mirrors `measureMessageRow` in `MessageList.tsx`, the same guard against
 * the same class of virtualizer for the same reason.
 */
function measureDiffItem(
  element: Element,
  entry: ResizeObserverEntry | undefined,
  instance: Virtualizer<HTMLDivElement, Element>,
): number {
  const index = instance.indexFromElement(element);
  const key = instance.options.getItemKey(index);
  const measured = measureElement(element, entry, instance);
  const cached = instance.itemSizeCache.get(key);
  if (cached !== undefined && measured <= cached) return cached;
  return measured > 0 ? measured : instance.options.estimateSize(index);
}

/** The one scrollable frame both the virtualized and plain row lists render
 * into -- same landmark, same bounds, same test id, differing only in the
 * scroll-element ref a virtualizer needs to size itself. */
function DiffScrollShell({
  children,
  mode,
  scrollRef,
}: {
  children: ReactNode;
  mode: DiffMode;
  scrollRef?: React.Ref<HTMLDivElement>;
}) {
  return (
    <div
      aria-label="File diff. Scroll to review all changes."
      className="max-h-[32rem] overflow-auto surface-card"
      data-testid={mode === "split" ? "split-diff" : "unified-diff"}
      ref={scrollRef}
      tabIndex={0}
    >
      {children}
    </div>
  );
}

function VirtualizedDiff({ items, mode }: { items: readonly RenderItem[]; mode: DiffMode }) {
  const [scrollElement, setScrollElement] = React.useState<HTMLDivElement | null>(null);
  const virtualizer = useVirtualizer({
    count: items.length,
    estimateSize: (index) => itemEstimate(items[index]),
    getItemKey: (index) => items[index]?.key ?? index,
    getScrollElement: () => scrollElement,
    measureElement: measureDiffItem,
    observeElementRect: observeScrollRect,
    overscan: 24,
  });

  return (
    <DiffScrollShell mode={mode} scrollRef={setScrollElement}>
      <div className="relative min-w-full" style={{ height: `${virtualizer.getTotalSize()}px` }}>
        {virtualizer.getVirtualItems().map((virtualItem) => {
          const item = items[virtualItem.index];
          return (
            <div
              className="absolute left-0 top-0 min-w-full"
              data-index={virtualItem.index}
              key={item.key}
              ref={virtualizer.measureElement}
              style={{ transform: `translateY(${virtualItem.start}px)` }}
            >
              {renderItem(item)}
            </div>
          );
        })}
      </div>
    </DiffScrollShell>
  );
}

function DiffItems({ items, mode }: { items: readonly RenderItem[]; mode: DiffMode }) {
  // Not keyed by `mode`: the two modes' rows carry different keys, and each row
  // is measured (a wrapped split row is taller than its one-line estimate), so
  // there is no correctness reason to remount the virtualizer -- and throwing
  // away its scroll position on every Unified/Split toggle undoes the reader's
  // place in a large diff for no reason.
  if (items.length > VIRTUALIZE_AFTER_ROWS) return <VirtualizedDiff items={items} mode={mode} />;
  return (
    <DiffScrollShell mode={mode}>
      {items.map((item) => (
        <React.Fragment key={item.key}>{renderItem(item)}</React.Fragment>
      ))}
    </DiffScrollShell>
  );
}

function buildItems(sections: readonly DiffSection[], mode: DiffMode): RenderItem[] {
  return sections.flatMap((section, sectionIndex): RenderItem[] => {
    if (section.kind === "opaque") {
      return [{ kind: "opaque", key: `${sectionIndex}:opaque`, content: section.content }];
    }
    const header: RenderItem = { kind: "header", key: `${sectionIndex}:header`, section };
    if (mode === "unified") {
      return [
        header,
        ...section.rows.map((row, rowIndex): RenderItem => ({
          kind: "unified",
          key: `${sectionIndex}:unified:${rowIndex}`,
          path: section.path,
          row,
        })),
      ];
    }
    return [
      header,
      ...toSplitRows(section.rows).map((row, rowIndex): RenderItem => ({
        kind: "split",
        key: `${sectionIndex}:split:${rowIndex}`,
        path: section.path,
        row,
      })),
    ];
  });
}

export function FileDiffViewer({ source, className, defaultMode = "unified" }: FileDiffViewerProps) {
  const parsed = useMemo(() => parseUnifiedDiff(source), [source]);
  const [mode, setMode] = useState<DiffMode>(defaultMode);
  const { sections, hasTextDiff } = useMemo(() => {
    const sections = parsed.kind === "sections" ? parsed.sections : [];
    return { sections, hasTextDiff: sections.some((section) => section.kind === "text") };
  }, [parsed]);
  const items = useMemo(() => buildItems(sections, mode), [mode, sections]);
  // Names the one file being reviewed, matching a screen reader's landmark
  // navigation to a single-file approval exactly as before this view learned
  // to show more than one file. A multi-file plan has no single name to
  // give the region, so it keeps the generic label.
  const soleTextPath = sections.length === 1 && sections[0].kind === "text" ? sections[0].path : null;
  const landmarkLabel = soleTextPath ? `Changes to ${soleTextPath}` : "File changes";

  if (parsed.kind === "opaque") {
    return (
      <section aria-label={landmarkLabel} className={cn("flex min-h-0 flex-col gap-2", className)}>
        <Alert title="Line preview unavailable" tone="warn">
          The raw change text is shown below instead.
        </Alert>
        <pre
          className="max-h-[32rem] overflow-auto whitespace-pre-wrap break-words surface-card p-4 font-mono text-xs text-secondary"
          tabIndex={0}
        >
          {parsed.source}
        </pre>
      </section>
    );
  }

  return (
    <section aria-label={landmarkLabel} className={cn("flex min-h-0 flex-col gap-2", className)}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-xs text-secondary">
          {sections.length} {sections.length === 1 ? "change section" : "change sections"}
        </p>
        {hasTextDiff ? (
          <SegmentedControl
            aria-label="Diff layout"
            onChange={setMode}
            options={[
              { value: "unified", label: "Unified", icon: <Rows3 aria-hidden size={14} /> },
              { value: "split", label: "Split", icon: <Columns2 aria-hidden size={14} /> },
            ]}
            value={mode}
          />
        ) : null}
      </div>
      {parsed.truncated ? (
        <Alert title="Preview limit reached" tone="warn">
          This diff is too large to show in full. The marker at the end shows where the preview stops.
        </Alert>
      ) : null}
      <DiffItems items={items} mode={mode} />
    </section>
  );
}
