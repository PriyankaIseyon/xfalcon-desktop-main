import type { ReactNode } from "react";
import { cn } from "../lib/cn";

export interface NavBarProps {
  children: ReactNode;
  className?: string;
  /** Sticky top chrome with vibrancy (maps to `.app-titlebar`). */
  variant?: "titlebar" | "plain";
}

export function NavBar({ children, className, variant = "titlebar" }: NavBarProps) {
  return (
    <header
      className={cn(variant === "titlebar" ? "app-titlebar" : "border-b border-hairline bg-card px-4 py-2", className)}
    >
      {children}
    </header>
  );
}
