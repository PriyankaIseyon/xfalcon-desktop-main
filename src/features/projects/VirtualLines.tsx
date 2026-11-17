import React from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { observeScrollRect } from "../../lib/virtualScrollRect";

/** Every line is one row of this height: lines never wrap, so the height is
 * known without measuring, and scrolling a million lines costs one layout. */
const LINE_HEIGHT_PX = 20;
/** Rows rendered past each edge of the viewport, so a fast scroll does not
 * show blank rows before the next frame. */
const OVERSCAN_LINES = 40;

export interface VirtualLinesProps {
  /** How many lines there are. */
  count: number;
  /** One line's content. Called only for visible lines. */
  renderLine: (index: number) => React.ReactNode;
  /** The number shown beside line 0, or `null` for no gutter — a window into
   * the middle of a file too large to count has no line numbers to show. */
  firstLineNumber: number | null;
  /** The widest line as drawn, in columns (`lineColumns` in `fileLines.tsx`),
   * so the rows are wide enough to scroll to its end. */
  maxLineColumns: number;
  label: string;
}

/**
 * A text of any length as a virtualized list of lines: only the lines in view
 * exist in the DOM (spec: project-file-editing).
 */
export function VirtualLines({ count, renderLine, firstLineNumber, maxLineColumns, label }: VirtualLinesProps) {
  const scrollRef = React.useRef<HTMLDivElement | null>(null);
  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => LINE_HEIGHT_PX,
    overscan: OVERSCAN_LINES,
    observeElementRect: observeScrollRect,
  });
  const gutterChars = firstLineNumber === null ? 0 : String(firstLineNumber + Math.max(count - 1, 0)).length + 2;

  return (
    <div
      aria-label={label}
      className="project-file-lines min-h-0 flex-1 overflow-auto surface-card font-mono text-xs text-foreground"
      ref={scrollRef}
      role="region"
      tabIndex={0}
    >
      <div
        className="relative"
        style={{
          height: `${virtualizer.getTotalSize()}px`,
          // `ch` of the monospace face: wide enough for the longest line, so
          // horizontal scroll reaches its end.
          width: `calc(${gutterChars + maxLineColumns}ch + 1.5rem)`,
          minWidth: "100%",
        }}
      >
        {virtualizer.getVirtualItems().map((item) => (
          <div
            className="project-file-lines__row absolute left-0 top-0 flex w-full"
            key={item.key}
            style={{ height: `${item.size}px`, transform: `translateY(${item.start}px)` }}
          >
            {firstLineNumber === null ? null : (
              <span
                aria-hidden
                className="project-file-lines__number shrink-0 select-none text-right text-faint"
                style={{ width: `${gutterChars}ch` }}
              >
                {firstLineNumber + item.index}
              </span>
            )}
            <span className="project-file-lines__text">{renderLine(item.index)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
