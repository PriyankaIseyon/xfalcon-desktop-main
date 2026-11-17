import React from "react";
import { cn } from "../lib/cn";

// ---------------------------------------------------------------------------
// Typographic role primitives — the single source of truth for heading styles
// so every surface renders the same hierarchy (per spec §4 type scale).
// Surfaces use these instead of hand-writing `text-* font-* text-*` per view,
// which is what caused cross-view drift. Change a role's look here, once.
//
//   HeroTitle    — the one hero heading on a chat-first empty state
//   PageTitle    — a tab/page heading      (`type-title`, 600)
//   SectionTitle — a card / group heading  (`type-heading`, 600)
//   SectionLabel — a small eyebrow/group label in panels (`type-caption`, 600, secondary)
//   Eyebrow      — small uppercase label above hero copy
//
// Each role owns its own size, line height, and tracking, so callers add weight
// and color only. `text-lg`/`text-base` were fixed px that ignored the
// reading-size setting, and `tracking-tight` fought the role's own letter
// spacing -- neither belongs on a role class.
//
// # Look and level are separate questions
//
// A role used to be welded to one element: `SectionTitle` was always an `<h3>`.
// A dialog title wants that exact look at `<h2>`, so `Modal` and `AlertDialog`
// each hand-wrote `type-heading font-semibold text-foreground` instead -- two
// copies of a role that exists, drifting from the day they were written. The
// role sets the LOOK; `level` sets the semantic depth, and each caller answers
// the question its own document outline asks.
// ---------------------------------------------------------------------------

export type HeadingLevel = 1 | 2 | 3 | 4 | 5 | 6;

export interface HeadingProps extends React.HTMLAttributes<HTMLHeadingElement> {
  /** Semantic depth in the document outline. Defaults to the role's usual place. */
  level?: HeadingLevel;
}

/**
 * Each role is written out rather than produced by a factory. A factory-made
 * component is invisible to Fast Refresh, which is the same reason
 * `menuContext.ts` and `useModalFocusTrap.ts` live in files of their own.
 */
function headingElement(level: HeadingLevel, role: string, className: string | undefined, props: object) {
  return React.createElement(`h${level}`, { className: cn(role, className), ...props });
}

export function HeroTitle({ level = 1, className, ...props }: HeadingProps) {
  return headingElement(level, "type-display font-semibold text-foreground", className, props);
}

export function PageTitle({ level = 2, className, ...props }: HeadingProps) {
  return headingElement(level, "type-title font-semibold text-foreground", className, props);
}

export function SectionTitle({ level = 3, className, ...props }: HeadingProps) {
  return headingElement(level, "type-heading font-semibold text-foreground", className, props);
}

export function SectionLabel({ level = 4, className, ...props }: HeadingProps) {
  return headingElement(level, "type-caption font-semibold text-secondary", className, props);
}

export interface EyebrowProps extends React.HTMLAttributes<HTMLElement> {
  /** Renders the eyebrow as a heading at this depth. Omit for the default
   * paragraph, which is what an eyebrow above hero copy is: a caption, not a
   * rung in the outline. */
  level?: HeadingLevel;
}

/** Small uppercase eyebrow above hero copy, and the group label a section wears
 * when that label IS the section's heading. */
export function Eyebrow({ level, className, ...props }: EyebrowProps) {
  return React.createElement(level == null ? "p" : `h${level}`, {
    className: cn("type-eyebrow mb-2 text-muted", className),
    ...props,
  });
}
