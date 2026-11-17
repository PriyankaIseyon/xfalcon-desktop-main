import type React from "react";
import { cn } from "../lib/cn";

export type SkeletonProps = React.HTMLAttributes<HTMLDivElement>;

export function Skeleton({ className, ...props }: SkeletonProps) {
  return <div {...props} aria-hidden className={cn("skeleton-shimmer rounded-md", className)} />;
}
