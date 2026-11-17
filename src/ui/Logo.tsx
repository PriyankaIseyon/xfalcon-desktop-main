import { cn } from "../lib/cn";
import { XFalconMark } from "./XFalconMark";

export interface LogoProps {
  showWordmark?: boolean;
  subtitle?: string;
  className?: string;
}

export function Logo({ showWordmark = true, subtitle, className }: LogoProps) {
  return (
    <div className={cn("flex items-center gap-2.5", className)}>
      <span aria-hidden className="app-mark">
        <XFalconMark />
      </span>
      {showWordmark ? (
        <span className="flex min-w-0 flex-col">
          <strong className="type-label tracking-tight text-foreground">
            <span className="logo-wordmark__x">x</span>Falcon
          </strong>
          {subtitle ? <small className="type-micro text-muted">{subtitle}</small> : null}
        </span>
      ) : null}
    </div>
  );
}
