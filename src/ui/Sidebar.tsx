import type { ReactNode } from "react";
import { cn } from "../lib/cn";

export interface SidebarProps {
  children: ReactNode;
  open?: boolean;
  className?: string;
  "aria-label"?: string;
}

export function Sidebar({ children, open = true, className, "aria-label": ariaLabel = "Sidebar" }: SidebarProps) {
  return (
    <aside aria-hidden={!open} aria-label={ariaLabel} className={cn("app-sidebar", className)} inert={!open}>
      <nav aria-label="Workspace" className="app-sidebar__content">
        {children}
      </nav>
    </aside>
  );
}
