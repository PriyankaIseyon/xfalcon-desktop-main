import type React from "react";
import { cn } from "../lib/cn";

export interface ProgressProps extends Omit<
  React.HTMLAttributes<HTMLDivElement>,
  "role" | "aria-label" | "aria-valuemax" | "aria-valuemin" | "aria-valuenow" | "aria-valuetext"
> {
  value: number;
  max?: number;
  label: string;
  indeterminate?: boolean;
  /** `success` reinforces completion. `label` must still name that state. */
  tone?: "accent" | "success";
}

export function Progress({
  value,
  max = 100,
  label,
  indeterminate = false,
  tone = "accent",
  className,
  ...props
}: ProgressProps) {
  const clamped = Math.min(max, Math.max(0, value));
  const percent = max > 0 ? (clamped / max) * 100 : 0;
  const state = indeterminate ? "indeterminate" : clamped >= max ? "complete" : "progress";

  return (
    <div
      {...props}
      aria-label={label}
      aria-valuemax={max}
      aria-valuemin={0}
      aria-valuenow={indeterminate ? undefined : clamped}
      aria-valuetext={indeterminate ? label : `${Math.round(percent)}%`}
      className={cn("h-1.5 w-full overflow-hidden rounded-pill bg-active", className)}
      data-state={state}
      role="progressbar"
    >
      <div
        className={cn(
          "h-full rounded-pill transition-[width] duration-slow ease-soft",
          tone === "success" ? "bg-success" : "bg-accent",
          indeterminate && "xf-progress-indeterminate w-1/3",
        )}
        style={indeterminate ? undefined : { width: `${percent}%` }}
      />
    </div>
  );
}
