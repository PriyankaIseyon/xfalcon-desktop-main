import React from "react";
import { Loader2 } from "lucide-react";
import { cn } from "../lib/cn";
import { type ButtonShape, type ButtonSize, type ButtonVariant, buttonClassName } from "./buttonClassName";

/**
 * Variant is hierarchy; shape is geometry. They are separate props on purpose:
 * a `pill` variant used to sit beside the `pill` shape, so "a pill button"
 * named two different things and neither one said which hierarchy it carried.
 *
 * `title` is deliberately absent from every action primitive: a native tooltip
 * is unstyled, keyboard-invisible, and a second grammar beside `Tooltip`. A
 * button with visible text needs no hint; one without takes `aria-label`,
 * which `IconButton` shows through `Tooltip`.
 */
export interface ButtonProps extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "title"> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  shape?: ButtonShape;
  loading?: boolean;
  /** Toggle / segmented selection (sets `aria-pressed`). */
  selected?: boolean;
}

export function Button({
  variant = "secondary",
  size = "md",
  shape = "default",
  type = "button",
  loading = false,
  selected,
  disabled,
  className,
  children,
  ...props
}: ButtonProps) {
  return (
    <button
      type={type}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      aria-pressed={selected === undefined ? undefined : selected}
      className={cn(buttonClassName({ variant, size, shape, selected }), className)}
      {...props}
    >
      {loading && <Loader2 className="animate-spin" aria-hidden />}
      {children}
    </button>
  );
}
