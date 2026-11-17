import React from "react";
import { cn } from "../lib/cn";

export interface CardProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Adds hover-lift + press scale + cursor-pointer for clickable cards. */
  interactive?: boolean;
}

export function Card({
  interactive = false,
  className,
  onClick,
  onKeyDown,
  role,
  tabIndex,
  ...props
}: CardProps) {
  // An interactive card must be operable by keyboard: expose it as a button and
  // activate onClick on Enter/Space (the global focus-visible ring targets
  // [role="button"], so it also becomes focus-visible).
  const a11y = interactive
    ? {
        role: role ?? "button",
        tabIndex: tabIndex ?? 0,
        onKeyDown: (e: React.KeyboardEvent<HTMLDivElement>) => {
          onKeyDown?.(e);
          if ((e.key === "Enter" || e.key === " ") && onClick) {
            e.preventDefault();
            onClick(e as unknown as React.MouseEvent<HTMLDivElement>);
          }
        },
      }
    : { role, tabIndex, onKeyDown };

  return (
    <div
      className={cn(
        "surface-card",
        interactive && "hover-lift press cursor-pointer",
        className,
      )}
      onClick={onClick}
      {...a11y}
      {...props}
    />
  );
}
