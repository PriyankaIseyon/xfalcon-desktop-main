import React from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { cn } from "../lib/cn";
import { IconButton } from "./IconButton";

export interface PaginationProps {
  page: number;
  pageCount: number;
  onPageChange: (page: number) => void;
  className?: string;
  /** The landmark's name. Two paginations on one page (above and below a table)
   * need two names, or a screen reader lists two landmarks it cannot tell apart. */
  label?: string;
}

/**
 * Page back and forward, with the position stated in words.
 *
 * # The position is announced
 *
 * "Page 2 of 5" used to change silently, so a keyboard reader pressed a control
 * and got no confirmation that anything had happened. It is a live region now,
 * and polite: the move is worth reporting, never worth interrupting.
 *
 * # Focus survives the last page
 *
 * Stepping to the last page disables the control that was just pressed, and a
 * disabled element cannot hold focus — the browser drops focus to `<body>` and
 * the next Tab restarts from the top of the document. Focus moves to the
 * control that is still live before that can happen.
 */
export function Pagination({ page, pageCount, onPageChange, className, label = "Pagination" }: PaginationProps) {
  const previousRef = React.useRef<HTMLButtonElement | null>(null);
  const nextRef = React.useRef<HTMLButtonElement | null>(null);
  const rescue = React.useRef<React.RefObject<HTMLButtonElement | null> | null>(null);
  const canPrevious = page > 1;
  const canNext = page < pageCount;

  // After the commit, not during the click: the control being left is still
  // enabled while the handler runs, and the one being moved to is still
  // disabled -- focusing a disabled element does nothing at all.
  React.useLayoutEffect(() => {
    const target = rescue.current?.current ?? null;
    rescue.current = null;
    if (target && !target.disabled) target.focus();
  }, [page]);

  const go = (target: number, endOfTheLine: boolean, fallback: React.RefObject<HTMLButtonElement | null>) => {
    rescue.current = endOfTheLine ? fallback : null;
    onPageChange(target);
  };

  return (
    <nav aria-label={label} className={cn("inline-flex items-center gap-2", className)}>
      <IconButton
        ref={previousRef}
        aria-label="Previous page"
        className="touch-target"
        disabled={!canPrevious}
        type="button"
        onClick={() => go(page - 1, page - 1 <= 1, nextRef)}
      >
        <ChevronLeft aria-hidden size={16} />
      </IconButton>
      <span aria-live="polite" className="type-caption tabular-nums text-secondary">
        Page <span className="font-medium text-foreground">{page}</span> of{" "}
        <span className="font-medium text-foreground">{pageCount}</span>
      </span>
      <IconButton
        ref={nextRef}
        aria-label="Next page"
        className="touch-target"
        disabled={!canNext}
        type="button"
        onClick={() => go(page + 1, page + 1 >= pageCount, previousRef)}
      >
        <ChevronRight aria-hidden size={16} />
      </IconButton>
    </nav>
  );
}
