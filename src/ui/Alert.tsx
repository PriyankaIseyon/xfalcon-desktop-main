import type { LucideIcon } from "lucide-react";
import { AlertCircle, AlertTriangle, CheckCircle2, Info } from "lucide-react";
import { cn } from "../lib/cn";

export type AlertTone = "info" | "success" | "warn" | "danger";

const toneConfig: Record<AlertTone, { icon: LucideIcon; className: string; role: "status" | "alert" }> = {
  info: { icon: Info, className: "border-default bg-surface text-secondary", role: "status" },
  success: { icon: CheckCircle2, className: "border-success bg-success-bg text-success", role: "status" },
  warn: { icon: AlertTriangle, className: "border-warn bg-warn-bg text-warn", role: "status" },
  danger: { icon: AlertCircle, className: "border-danger bg-danger-bg text-danger", role: "alert" },
};

export interface AlertProps extends Omit<React.HTMLAttributes<HTMLDivElement>, "role"> {
  tone?: AlertTone;
  title?: string;
  /** A recovery or next-step control. It stays in the same place for every tone. */
  action?: React.ReactNode;
}

export function Alert({ tone = "info", title, action, className, children, ...props }: AlertProps) {
  const config = toneConfig[tone];
  const Icon = config.icon;

  return (
    <div
      {...props}
      className={cn("flex gap-2.5 rounded-lg border px-3 py-2.5 text-sm leading-relaxed", config.className, className)}
      data-tone={tone}
      role={config.role}
    >
      <Icon aria-hidden className="mt-0.5 shrink-0" size={16} strokeWidth={1.8} />
      <div className="min-w-0 flex-1">
        {title ? <p className="mb-0.5 font-semibold">{title}</p> : null}
        {children}
      </div>
      {action ? <div className="shrink-0 self-start">{action}</div> : null}
    </div>
  );
}
