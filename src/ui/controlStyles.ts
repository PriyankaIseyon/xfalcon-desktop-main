import { cn } from "../lib/cn";

/** Shared field control surface — inputs, selects, search, native date/time/file triggers. */
export const controlBase = cn(
  "touch-target w-full bg-card border border-default rounded-md px-3 text-sm text-foreground",
  "placeholder:text-faint transition-colors duration-fast ease-soft",
  "hover:border-strong",
  "focus:outline-none focus:border-accent focus:ring-2 focus:ring-accent-muted",
  "disabled:cursor-not-allowed disabled:opacity-50",
  "aria-invalid:border-danger aria-invalid:focus:border-danger aria-invalid:focus:ring-danger-bg",
);

export const controlSizes = {
  md: "h-control-default",
  lg: "h-control-prominent",
} as const;
