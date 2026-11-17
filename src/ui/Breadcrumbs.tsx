import React from "react";
import { ChevronRight } from "lucide-react";
import { cn } from "../lib/cn";
import { Link } from "./Link";

/** One recipe for both kinds of crumb -- the `nav` link variant, spelled out
 * here for the button that cannot be a `Link`. */
/** The hit halo is for coarse pointers: a crumb is a line of text, and a line
 * of text is under 44 px tall however it is styled. */
const TRAIL_ITEM =
  "touch-target--halo inline-flex items-center gap-1 rounded-sm text-muted transition-colors hover:text-foreground";

export interface BreadcrumbItem {
  label: string;
  href?: string;
  onClick?: () => void;
}

export interface BreadcrumbsProps {
  items: readonly BreadcrumbItem[];
  className?: string;
}

export function Breadcrumbs({ items, className }: BreadcrumbsProps) {
  return (
    <nav aria-label="Breadcrumb" className={className}>
      <ol className="flex flex-wrap items-center gap-1 type-caption text-muted">
        {items.map((item, index) => {
          const isLast = index === items.length - 1;
          return (
            <li key={`${item.label}-${index}`} className="inline-flex items-center gap-1">
              {index > 0 ? <ChevronRight aria-hidden className="text-faint" size={12} /> : null}
              {isLast ? (
                <span aria-current="page" className="font-medium text-foreground">
                  {item.label}
                </span>
              ) : item.href ? (
                <Link className="touch-target--halo" href={item.href} variant="nav">
                  {item.label}
                </Link>
              ) : (
                // A crumb that acts on the app rather than addressing a place
                // is a button, and it wears the trail's own link recipe so the
                // two kinds of crumb are indistinguishable to the eye.
                <button className={cn(TRAIL_ITEM, "cursor-pointer")} type="button" onClick={item.onClick}>
                  {item.label}
                </button>
              )}
            </li>
          );
        })}
      </ol>
    </nav>
  );
}
