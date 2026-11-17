import { cn } from "../lib/cn";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "destructive";
export type ButtonSize = "sm" | "md" | "lg";
export type ButtonShape = "default" | "pill";

const base =
  "press touch-target inline-flex items-center justify-center gap-2 font-medium select-none disabled:opacity-50 disabled:pointer-events-none";

const variants: Record<ButtonVariant, string> = {
  primary: "bg-accent text-accent-ink hover:bg-accent-fill-hover active:bg-accent-fill-active shadow-e1",
  secondary:
    "bg-card text-foreground border border-default hover:bg-hover hover:border-strong active:bg-active shadow-e1",
  ghost: "bg-transparent text-secondary hover:bg-hover hover:text-foreground active:bg-active",
  // Same bordered box as `secondary` -- a destructive trigger sitting beside
  // "Edit"/"Sign out" in a card's action row must still read as a button, not
  // as bare red text next to two boxed ones.
  danger: "bg-transparent text-danger border border-default hover:border-danger/60 hover:bg-danger-bg",
  destructive: "bg-danger text-danger-ink border border-danger hover:opacity-90 shadow-e1",
};

const shapes: Record<ButtonShape, string> = {
  default: "rounded-md",
  pill: "rounded-full",
};

const sizes: Record<ButtonSize, string> = {
  sm: "h-control-compact px-3 text-xs control-glyph-compact",
  md: "h-control-default px-3 text-xs control-glyph-default",
  lg: "h-control-prominent px-4 text-sm control-glyph-prominent",
};

/**
 * The class list a `Button` of this variant, size, and shape wears. Exported
 * for the one control that cannot be a `Button` element's caller -- an action a
 * foreign component renders into a slot -- so it still looks like every other
 * button instead of carrying a hand-kept copy of these styles.
 */
export function buttonClassName({
  variant = "secondary",
  size = "md",
  shape = "default",
  selected,
}: { variant?: ButtonVariant; size?: ButtonSize; shape?: ButtonShape; selected?: boolean } = {}): string {
  return cn(
    base,
    shapes[shape],
    variants[variant],
    sizes[size],
    selected && "bg-active text-foreground shadow-e1 hover:bg-active active:bg-active",
  );
}
