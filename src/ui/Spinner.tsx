import { Loader2 } from "lucide-react";
import { cn } from "../lib/cn";

interface SpinnerBaseProps {
  className?: string;
  size?: number;
}

export type SpinnerProps = SpinnerBaseProps &
  (
    | {
        /** Exact working copy for a standalone status indicator. */
        label: string;
        decorative?: false;
      }
    | {
        /** When true, omits the status landmark (use when a parent already exposes progress). */
        decorative: true;
        label?: never;
      }
  );

/** Accessible loading indicator — pairs with `aria-busy` on the parent when inline. */
export function Spinner({ className, size = 20, label, decorative = false }: SpinnerProps) {
  if (decorative) {
    return <Loader2 aria-hidden className={cn("animate-spin text-muted", className)} size={size} strokeWidth={1.8} />;
  }

  return (
    <span className={cn("inline-flex items-center justify-center text-muted", className)} role="status">
      <Loader2 aria-hidden className="animate-spin" size={size} strokeWidth={1.8} />
      <span className="sr-only">{label}</span>
    </span>
  );
}
