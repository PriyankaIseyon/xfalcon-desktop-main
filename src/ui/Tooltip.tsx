import React from "react";
import ReactDOM from "react-dom";
import { cn } from "../lib/cn";
import { useExitAnimation } from "./useExitAnimation";

export type TooltipSide = "top" | "bottom" | "left" | "right";

export interface TooltipProps {
  label: React.ReactNode;
  children: React.ReactElement;
  side?: TooltipSide;
  delay?: number;
  className?: string;
}

function calculatePosition(
  anchorRect: DOMRect,
  tooltipElement: HTMLElement,
  preferredSide: TooltipSide,
): { top: number; left: number; side: TooltipSide } {
  const margin = 8;
  const gap = 6;
  const tipWidth = tooltipElement.offsetWidth;
  const tipHeight = tooltipElement.offsetHeight;

  let top = 0;
  let left = 0;
  let side = preferredSide;

  // Collision detection against viewport boundaries
  if (preferredSide === "top" && anchorRect.top - tipHeight - gap < margin) {
    side = "bottom";
  } else if (preferredSide === "bottom" && anchorRect.bottom + tipHeight + gap > window.innerHeight - margin) {
    side = "top";
  } else if (preferredSide === "left" && anchorRect.left - tipWidth - gap < margin) {
    side = "right";
  } else if (preferredSide === "right" && anchorRect.right + tipWidth + gap > window.innerWidth - margin) {
    side = "left";
  }

  switch (side) {
    case "top":
      top = anchorRect.top - tipHeight - gap;
      left = anchorRect.left + (anchorRect.width - tipWidth) / 2;
      break;
    case "bottom":
      top = anchorRect.bottom + gap;
      left = anchorRect.left + (anchorRect.width - tipWidth) / 2;
      break;
    case "left":
      top = anchorRect.top + (anchorRect.height - tipHeight) / 2;
      left = anchorRect.left - tipWidth - gap;
      break;
    case "right":
      top = anchorRect.top + (anchorRect.height - tipHeight) / 2;
      left = anchorRect.right + gap;
      break;
  }

  // Viewport clamp
  left = Math.max(margin, Math.min(left, window.innerWidth - tipWidth - margin));
  top = Math.max(margin, Math.min(top, window.innerHeight - tipHeight - margin));

  return { top, left, side };
}

/** A pointer that lands on a control focuses it. Without this check the click
 * that dismisses a menu also parks a tooltip over the thing it just acted on.
 * jsdom versions that do not know the selector throw, and a tooltip that shows
 * too eagerly is the safer failure. */
function isKeyboardFocus(node: Element): boolean {
  try {
    return node.matches(":focus-visible");
  } catch {
    return true;
  }
}

/**
 * The product's one hover-and-focus help contract.
 *
 * Renders through a portal so a tooltip is never clipped by an `overflow`
 * ancestor, flips to the opposite side when the preferred one would leave the
 * viewport, and closes on Escape, scroll, and resize.
 *
 * The child carries the anchor ref directly. An earlier version wrapped it in
 * an `inline-flex` span, which quietly made `<Tooltip><IconButton/></Tooltip>`
 * lay out differently from a bare `<IconButton/>` in the same flex row.
 */
export function Tooltip({ label, children, side = "top", delay = 300, className }: TooltipProps) {
  const id = React.useId();
  const tooltipId = `tooltip-${id}`;
  const timerRef = React.useRef<number | undefined>(undefined);
  const anchorRef = React.useRef<HTMLElement | null>(null);
  const panelRef = React.useRef<HTMLSpanElement | null>(null);
  const exitRef = useExitAnimation(panelRef);
  const [open, setOpen] = React.useState(false);

  const clearTimer = React.useCallback(() => {
    if (timerRef.current !== undefined) {
      window.clearTimeout(timerRef.current);
      timerRef.current = undefined;
    }
  }, []);

  const show = React.useCallback(
    (immediate = false) => {
      clearTimer();
      if (immediate) {
        setOpen(true);
      } else {
        timerRef.current = window.setTimeout(() => setOpen(true), delay);
      }
    },
    [clearTimer, delay],
  );

  const hide = React.useCallback(() => {
    clearTimer();
    setOpen(false);
  }, [clearTimer]);

  React.useEffect(() => clearTimer, [clearTimer]);

  React.useLayoutEffect(() => {
    if (!open || !anchorRef.current || !panelRef.current) return;
    const anchorRect = anchorRef.current.getBoundingClientRect();
    const pos = calculatePosition(anchorRect, panelRef.current, side);
    panelRef.current.style.top = `${pos.top}px`;
    panelRef.current.style.left = `${pos.left}px`;
    // The side it landed on after the flip, which is where it drifts in from.
    panelRef.current.dataset.side = pos.side;
    panelRef.current.style.visibility = "visible";
  }, [open, side]);

  React.useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") hide();
    };
    const onScroll = () => hide();
    document.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);
    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
    };
  }, [hide, open]);

  const childProps = children.props as {
    ref?: React.Ref<HTMLElement>;
    "aria-label"?: string;
    "aria-describedby"?: string;
    onMouseEnter?: (event: React.MouseEvent) => void;
    onMouseLeave?: (event: React.MouseEvent) => void;
    onPointerDown?: (event: React.PointerEvent) => void;
    onFocus?: (event: React.FocusEvent) => void;
    onBlur?: (event: React.FocusEvent) => void;
  };

  const childRef = childProps.ref;
  const setAnchor = React.useCallback(
    (node: HTMLElement | null) => {
      anchorRef.current = node;
      if (typeof childRef === "function") childRef(node);
      else if (childRef) (childRef as React.RefObject<HTMLElement | null>).current = node;
    },
    [childRef],
  );

  // A tooltip that only repeats the accessible name is not a description. Left
  // described-by, a screen reader reads "Copy message, Copy message".
  const repeatsName = typeof label === "string" && childProps["aria-label"] === label;
  const describedBy =
    open && !repeatsName
      ? [childProps["aria-describedby"], tooltipId].filter(Boolean).join(" ")
      : childProps["aria-describedby"];

  const child = React.cloneElement(children, {
    ref: setAnchor,
    "aria-describedby": describedBy,
    onMouseEnter: (event: React.MouseEvent) => {
      childProps.onMouseEnter?.(event);
      show(false);
    },
    onMouseLeave: (event: React.MouseEvent) => {
      childProps.onMouseLeave?.(event);
      hide();
    },
    onPointerDown: (event: React.PointerEvent) => {
      childProps.onPointerDown?.(event);
      hide();
    },
    onFocus: (event: React.FocusEvent) => {
      childProps.onFocus?.(event);
      if (isKeyboardFocus(event.currentTarget)) show(true);
    },
    onBlur: (event: React.FocusEvent) => {
      childProps.onBlur?.(event);
      hide();
    },
  } as React.HTMLAttributes<HTMLElement> & { ref: React.Ref<HTMLElement> });

  const portalContent =
    open && typeof document !== "undefined"
      ? ReactDOM.createPortal(
          <span
            ref={exitRef}
            role="tooltip"
            id={tooltipId}
            // Pinned to the origin until the layout effect measures it, so the
            // pre-measure frame cannot push the document wider.
            className={cn(
              "xf-material xf-overlay-fade pointer-events-none invisible fixed left-0 top-0 z-[var(--xf-z-tooltip)] whitespace-nowrap rounded-md px-2 py-1 text-xs text-foreground shadow-e2",
              className,
            )}
          >
            {label}
          </span>,
          document.body,
        )
      : null;

  return (
    <>
      {child}
      {portalContent}
    </>
  );
}
