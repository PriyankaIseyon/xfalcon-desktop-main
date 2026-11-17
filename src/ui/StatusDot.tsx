import React from "react";
import { cn } from "../lib/cn";

export type StatusDotTone = "online" | "offline" | "busy" | "error" | "accent";

const toneClasses: Record<StatusDotTone, string> = {
  online: "bg-success",
  offline: "bg-faint",
  busy: "bg-warn",
  error: "bg-danger",
  accent: "bg-accent",
};

interface StatusDotBaseProps
  extends Omit<React.HTMLAttributes<HTMLSpanElement>, "style" | "role" | "aria-label" | "aria-hidden"> {
  tone?: StatusDotTone;
  pulse?: boolean;
}

export type StatusDotProps = StatusDotBaseProps &
  (
    | { label: string; decorative?: false }
    | {
        /** A parent must state the same status in text. */
        decorative: true;
        label?: never;
      }
  );

export function StatusDot({
  tone = "offline",
  pulse = false,
  label,
  decorative = false,
  className,
  ...props
}: StatusDotProps) {
  const markerClassName = cn(
    "inline-block size-[7px] shrink-0 rounded-full",
    toneClasses[tone],
    pulse && "animate-pulse-soft",
  );

  if (decorative) {
    return <span {...props} aria-hidden="true" className={cn(markerClassName, className)} />;
  }

  return (
    <span
      {...props}
      aria-label={label}
      className={cn("inline-flex items-center gap-1.5", className)}
      role="img"
    >
      <span aria-hidden="true" className={markerClassName} />
      <span className="type-caption">{label}</span>
    </span>
  );
}
