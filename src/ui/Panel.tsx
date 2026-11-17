import React from "react";
import { cn } from "../lib/cn";

export interface PanelProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Add default padding (p-4) to the panel. */
  padded?: boolean;
}

export function Panel({ padded = false, className, ...props }: PanelProps) {
  return (
    <div
      className={cn("bg-surface", padded && "p-4", className)}
      {...props}
    />
  );
}
