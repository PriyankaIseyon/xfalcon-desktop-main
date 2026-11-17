import React from "react";
import { cn } from "../lib/cn";

export type BadgeTone = "neutral" | "accent" | "success" | "warn" | "danger";

const toneClasses: Record<BadgeTone, string> = {
  neutral: "bg-surface    text-secondary",
  accent:  "bg-accent-muted text-accent-hover",
  success: "bg-success-bg  text-success",
  warn:    "bg-warn-bg     text-warn",
  danger:  "bg-danger-bg   text-danger",
};

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  tone?: BadgeTone;
}

export function Badge({ tone = "neutral", className, ...props }: BadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-pill px-2 py-0.5 text-2xs font-medium leading-none",
        toneClasses[tone],
        className,
      )}
      {...props}
    />
  );
}
