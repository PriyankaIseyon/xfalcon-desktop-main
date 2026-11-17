import React from "react";
import { cn } from "../lib/cn";

/**
 * Attaches the full string only when the text is really clipped.
 *
 * Module scope on purpose: a virtualized grid renders hundreds of cells, and a
 * per-cell closure is a per-cell allocation on every scroll frame.
 */
function revealWhenClipped(event: React.PointerEvent<HTMLElement>) {
  const node = event.currentTarget;
  if (node.scrollWidth > node.clientWidth || node.dataset.abbreviated !== undefined) {
    node.title = node.dataset.fullText ?? node.textContent ?? "";
  } else {
    node.removeAttribute("title");
  }
}

export interface TruncatedTextProps extends Omit<React.HTMLAttributes<HTMLElement>, "title"> {
  /** The element to render. Clipped text is never a control, so the set is small. */
  as?: "span" | "strong" | "div" | "p";
  /** The whole string, when the rendered children are not it. */
  text?: string;
  /** The children are a shortened form of `text` (a name cut in the middle),
   * so the whole string is revealed even when what is shown fits. */
  abbreviated?: boolean;
}

/**
 * One-line text that reveals itself on hover when it does not fit.
 *
 * This is deliberately not `Tooltip`. A truncation reveal repeats text the
 * user can already see, so it is not a description; it has to survive hundreds
 * of rows in a virtualized list; and it must not take focus away from the row
 * that owns it. The platform `title` is the right mechanism for that job. The
 * defect it fixes is that a fixed `title` fires even when the text fits, so
 * every row grew a redundant hover panel -- measuring on pointer entry costs
 * nothing until a pointer arrives, and needs no per-row observer.
 */
export function TruncatedText({
  as: Tag = "span",
  text,
  abbreviated = false,
  className,
  onPointerEnter,
  children,
  ...props
}: TruncatedTextProps) {
  return (
    <Tag
      {...props}
      className={cn("truncate", className)}
      data-abbreviated={abbreviated ? "" : undefined}
      data-full-text={text}
      onPointerEnter={
        onPointerEnter
          ? (event: React.PointerEvent<HTMLElement>) => {
              revealWhenClipped(event);
              onPointerEnter(event);
            }
          : revealWhenClipped
      }
    >
      {children}
    </Tag>
  );
}
