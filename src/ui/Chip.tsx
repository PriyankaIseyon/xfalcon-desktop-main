import React from "react";
import { cn } from "../lib/cn";

const CHIP_BASE = cn(
  "inline-flex max-w-full items-center gap-1.5 rounded-pill border border-default px-3 py-1.5",
  "text-left text-xs font-medium leading-snug text-secondary transition-colors duration-fast ease-soft",
);

export type ChipProps =
  | ({ interactive?: true } & Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "title">)
  /**
   * A static chip is a `<span>`, so it must not be offered the props of a
   * control. The single prop type let `onClick` and `disabled` through to a
   * span, where a click handler is unreachable by keyboard and `disabled` is
   * simply dropped -- a control that looks live and answers nothing.
   */
  | ({ interactive: false } & Omit<React.HTMLAttributes<HTMLSpanElement>, "title">);

export function Chip({ interactive = true, className, children, ...props }: ChipProps) {
  if (interactive === false) {
    return (
      <span className={cn(CHIP_BASE, "bg-surface-alt text-muted", className)} {...props}>
        {children}
      </span>
    );
  }

  const { type = "button", disabled, ...buttonProps } = props as React.ButtonHTMLAttributes<HTMLButtonElement>;
  return (
    <button
      className={cn(
        CHIP_BASE,
        // `cursor-not-allowed`, like every other disabled control in the app.
        // This one said `cursor-default`, so a disabled chip was the one place
        // the cursor gave no answer.
        "touch-target press cursor-pointer bg-card hover:border-strong hover:bg-hover hover:text-foreground",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      disabled={disabled}
      type={type}
      {...buttonProps}
    >
      {children}
    </button>
  );
}
