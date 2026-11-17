import React from "react";
import { Loader2 } from "lucide-react";
import { cn } from "../lib/cn";
import { Tooltip, type TooltipSide } from "./Tooltip";

type IconButtonSize = "sm" | "md" | "lg";
/**
 * Hierarchy, on the same axis `Button` calls `variant` — because it is the
 * same axis. `tone` is reserved for the status vocabulary that `Badge`,
 * `Alert`, `Toast`, `StatusDot`, and `Progress` share (success, warn, error,
 * info, neutral); an action's hierarchy is not a status, and one word meaning
 * both left `danger` sitting in two unrelated vocabularies.
 */
type IconButtonVariant = "default" | "danger" | "primary";
type IconButtonShape = "default" | "circle";

const sizeClasses: Record<IconButtonSize, string> = {
  sm: "size-control-compact control-glyph-compact",
  md: "size-control-default control-glyph-default",
  lg: "size-control-prominent control-glyph-prominent",
};

/** Quiet until touched, then the app's one danger color -- so a Trash icon
 * reads as destructive the same way in a report row, a settings row, or a
 * chat sidebar, instead of each place inventing its own hover color. */
const variantClasses: Record<IconButtonVariant, string> = {
  default: "text-muted hover:text-foreground hover:bg-hover active:bg-active",
  danger: "text-muted hover:text-danger hover:bg-danger-bg active:bg-danger-bg",
  primary: "bg-accent text-accent-ink hover:bg-accent-fill-hover active:bg-accent-fill-active",
};

const shapeClasses: Record<IconButtonShape, string> = {
  default: "rounded-md",
  circle: "rounded-full",
};

export interface IconButtonProps
  extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "style" | "title"> {
  size?: IconButtonSize;
  selected?: boolean;
  /** Hierarchy of the action, on `Button`'s vocabulary. */
  variant?: IconButtonVariant;
  /** Shape: default rounded-md or circular. */
  shape?: IconButtonShape;
  /** Loading / busy indicator state. */
  loading?: boolean;
  /**
   * Help shown on hover and keyboard focus. Defaults to `aria-label`, so an
   * icon-only control is discoverable by sight as well as by screen reader
   * without every call site repeating itself. Pass a different string when the
   * hint says more than the name; pass `null` to suppress it.
   */
  tooltip?: React.ReactNode;
  /** Side placement for tooltip if provided. */
  tooltipSide?: TooltipSide;
  /** React 19 passes `ref` as an ordinary prop, and an action primitive that
   * cannot be referenced cannot be focused by the surface that owns it. */
  ref?: React.Ref<HTMLButtonElement>;
}

export function IconButton({
  size = "md",
  type = "button",
  selected,
  variant = "default",
  shape = "default",
  loading = false,
  disabled,
  tooltip,
  tooltipSide = "top",
  className,
  children,
  ref,
  ...props
}: IconButtonProps) {
  const label = tooltip === undefined ? props["aria-label"] : tooltip;
  const button = (
    <button
      ref={ref}
      type={type}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      aria-pressed={selected === undefined ? undefined : selected}
      className={cn(
        // No local transition declaration: `press` already owns transform,
        // colour, background, border, and shadow on the one duration and
        // curve. A `transition-colors` beside it narrowed the property list to
        // colour alone -- and, being a layered utility under an unlayered
        // rule, never applied either way.
        "press touch-target inline-flex items-center justify-center select-none disabled:opacity-50 disabled:pointer-events-none",
        shapeClasses[shape],
        sizeClasses[size],
        variantClasses[variant],
        selected && "bg-active text-foreground hover:bg-active active:bg-active",
        className,
      )}
      {...props}
    >
      {loading ? <Loader2 className="animate-spin" aria-hidden /> : children}
    </button>
  );

  if (label == null || label === "") return button;

  return (
    <Tooltip label={label} side={tooltipSide}>
      {button}
    </Tooltip>
  );
}
