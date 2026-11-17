import React from "react";
import { cn } from "../lib/cn";

type AvatarSize = "sm" | "md" | "lg";

const sizeClasses: Record<AvatarSize, string> = {
  sm: "size-7 text-2xs",
  md: "size-9 text-xs",
  lg: "size-11 text-sm",
};

export interface AvatarProps extends React.HTMLAttributes<HTMLSpanElement> {
  size?: AvatarSize;
}

/** Circular identity plate for menus, lists, and crew cards. */
export function Avatar({ size = "md", className, children, ...props }: AvatarProps) {
  return (
    <span
      className={cn(
        "inline-grid shrink-0 place-items-center rounded-full border border-default bg-accent-muted font-semibold text-accent-hover",
        sizeClasses[size],
        className,
      )}
      {...props}
    >
      {children}
    </span>
  );
}
