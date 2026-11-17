import React from "react";
import { cn } from "../lib/cn";

export interface TableProps extends React.TableHTMLAttributes<HTMLTableElement> {
  /**
   * Drops the card the table normally sits in, for a table that *is* the
   * surface rather than one panel on it.
   *
   * A card reads as one section among several. A destination whose whole
   * content is a list has no other section for it to be among, so the frame
   * only draws a box around the page. The parent owns scrolling in that case,
   * which is also what lets the header stick.
   */
  flush?: boolean;
}

export function Table({ className, flush = false, ...props }: TableProps) {
  const table = <table className={cn("w-full border-collapse text-left text-sm", className)} {...props} />;
  if (flush) return table;
  return <div className="overflow-x-auto surface-card">{table}</div>;
}

export interface TableHeadProps extends React.HTMLAttributes<HTMLTableSectionElement> {
  /** Keeps the header row visible while the rows scroll under it. Needs a
   * scrolling ancestor, which a `flush` table's parent is. */
  sticky?: boolean;
}

export function TableHead({ className, sticky = false, ...props }: TableHeadProps) {
  return (
    <thead
      className={cn(
        "table-heading border-b border-hairline",
        // On the cells rather than the section: a `thead` cannot be
        // positioned, so the offset has to land on the `th`s inside it. The
        // header keeps an opaque, distinct surface so scrolling rows cannot
        // bleed through its labels.
        sticky && "[&_th]:sticky [&_th]:top-0 [&_th]:z-10 [&_th]:bg-[var(--xf-table-header)]",
        className,
      )}
      {...props}
    />
  );
}

export function TableBody(props: React.HTMLAttributes<HTMLTableSectionElement>) {
  return <tbody {...props} />;
}

export interface TableRowProps extends React.HTMLAttributes<HTMLTableRowElement> {
  /** Highlights on hover. Only set this when the row itself is clickable --
   * every current row in this app opens or acts through a control inside a
   * cell, not the row, so a hover highlight with no `onClick` would invite a
   * click that does nothing. */
  interactive?: boolean;
}

export function TableRow({ interactive = false, className, ...props }: TableRowProps) {
  return (
    <tr
      className={cn(
        "border-b border-hairline transition-colors last:border-b-0",
        interactive && "cursor-pointer hover:bg-hover",
        className,
      )}
      {...props}
    />
  );
}

export function Th({ className, ...props }: React.ThHTMLAttributes<HTMLTableCellElement>) {
  return (
    <th
      className={cn("type-micro px-3 py-2 font-semibold uppercase tracking-wider text-muted", className)}
      {...props}
    />
  );
}

export function Td({ className, ...props }: React.TdHTMLAttributes<HTMLTableCellElement>) {
  return <td className={cn("px-3 py-2.5 text-secondary", className)} {...props} />;
}
