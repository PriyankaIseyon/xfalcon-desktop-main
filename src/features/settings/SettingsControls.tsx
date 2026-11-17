import React from "react";
import { Loader2, Trash2 } from "lucide-react";
import { cn } from "../../lib/cn";
import { SegmentedControl, type SegmentedControlProps } from "../../ui/SegmentedControl";
import { Tooltip } from "../../ui/Tooltip";

export type SettingsControlVariant = "primary" | "secondary" | "ghost" | "danger";

function SettingsControlIcon({ children }: { children: React.ReactNode }) {
  return (
    <span
      aria-hidden="true"
      className="settings-control__icon"
      data-settings-control-icon=""
      data-settings-icon-size="16"
    >
      {children}
    </span>
  );
}

export interface SettingsButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: SettingsControlVariant;
  icon?: React.ReactNode;
  loading?: boolean;
  /**
   * Labelled buttons in a settings row share one of two widths so a column of
   * them lines up: the regular width, and this wider one for a label that does
   * not fit the regular one ("Export diagnostics report"). Both are minimums.
   */
  wide?: boolean;
}

export function SettingsButton({
  variant = "secondary",
  icon,
  wide = false,
  loading = false,
  disabled,
  type = "button",
  className,
  children,
  ...props
}: SettingsButtonProps) {
  return (
    <button
      type={type}
      aria-busy={loading || undefined}
      disabled={disabled || loading}
      className={cn(
        "settings-control settings-control--radius settings-button",
        wide && "settings-button--wide",
        className,
      )}
      data-settings-control="button"
      data-settings-control-size="32"
      data-settings-variant={variant}
      {...props}
    >
      {loading ? (
        <SettingsControlIcon>
          <Loader2 className="settings-control__spinner" />
        </SettingsControlIcon>
      ) : icon ? (
        <SettingsControlIcon>{icon}</SettingsControlIcon>
      ) : null}
      {children}
    </button>
  );
}

export function SettingsRemoveButton(props: Omit<SettingsButtonProps, "children" | "icon" | "variant">) {
  return (
    <SettingsButton {...props} icon={<Trash2 />} variant="danger">
      Remove
    </SettingsButton>
  );
}

export interface SettingsIconButtonProps extends Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  "aria-label" | "children" | "title"
> {
  "aria-label": string;
  icon: React.ReactNode;
  variant?: SettingsControlVariant;
  loading?: boolean;
  /** Help on hover and keyboard focus. Defaults to `aria-label`. */
  tooltip?: React.ReactNode;
}

export function SettingsIconButton({
  "aria-label": ariaLabel,
  icon,
  variant = "ghost",
  loading = false,
  disabled,
  tooltip,
  type = "button",
  className,
  ...props
}: SettingsIconButtonProps) {
  return (
    <Tooltip label={tooltip ?? ariaLabel}>
      <button
        type={type}
        aria-label={ariaLabel}
        aria-busy={loading || undefined}
        disabled={disabled || loading}
        className={cn("settings-control settings-control--radius settings-icon-button", className)}
        data-settings-control="icon-button"
        data-settings-control-size="32"
        data-settings-variant={variant}
        {...props}
      >
        <SettingsControlIcon>{loading ? <Loader2 className="settings-control__spinner" /> : icon}</SettingsControlIcon>
      </button>
    </Tooltip>
  );
}

type AccessibleRadiogroupName =
  { "aria-label": string; "aria-labelledby"?: never } | { "aria-label"?: never; "aria-labelledby": string };

export type SettingsSegmentedControlProps<T extends string = string> = Omit<
  SegmentedControlProps<T>,
  "variant" | "aria-label" | "aria-labelledby"
> &
  AccessibleRadiogroupName;

export function SettingsSegmentedControl<T extends string = string>(props: SettingsSegmentedControlProps<T>) {
  return <SegmentedControl<T> variant="settings" {...props} />;
}
