import type { ReactNode } from "react";
import { cn } from "../lib/cn";
import { Eyebrow, HeroTitle } from "./Heading";

export interface EmptyStateProps {
  eyebrow?: ReactNode;
  title: ReactNode;
  /** Explains why this place is empty and points to the next useful action. */
  description: ReactNode;
  icon?: ReactNode;
  actions?: ReactNode;
  suggestions?: ReactNode;
  /** `center` for an empty state that is the whole view — nothing else is on
   * screen to align a left edge to, so the block centres itself and its text.
   * `start` where the empty state sits above or beside other content. */
  align?: "start" | "center";
  className?: string;
}

export function EmptyState({
  eyebrow,
  title,
  description,
  icon,
  actions,
  suggestions,
  align = "start",
  className,
}: EmptyStateProps) {
  const centered = align === "center";
  return (
    <div className={cn("max-w-xl animate-view", centered && "flex flex-col items-center text-center", className)}>
      {icon ? <div className="mb-4 text-muted">{icon}</div> : null}
      {eyebrow ? <Eyebrow>{eyebrow}</Eyebrow> : null}
      {typeof title === "string" ? <HeroTitle>{title}</HeroTitle> : title}
      {description ? <p className="type-body mt-3 max-w-lg text-secondary">{description}</p> : null}
      {suggestions ? <div className="mt-6">{suggestions}</div> : null}
      {actions ? <div className={cn("mt-6 flex flex-wrap gap-2", centered && "justify-center")}>{actions}</div> : null}
    </div>
  );
}
