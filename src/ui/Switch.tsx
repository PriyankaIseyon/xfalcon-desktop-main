import React from "react";
import { cn } from "../lib/cn";

// ---------------------------------------------------------------------------
// Switch — iOS-style toggle. Track turns from neutral (`bg-active`) to accent
// (`bg-accent`) when on; a `bg-card` knob slides via `transform` with the
// spring ease. `role="switch"` + `aria-checked`; Space/Enter toggle. When a
// `label` is supplied the control is wrapped in a <label> so a click anywhere
// toggles it and the text is programmatically associated.
// ---------------------------------------------------------------------------

export interface SwitchProps {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  /** Work is in flight. Busy switches stay named, stable, and inoperable. */
  busy?: boolean;
  label?: React.ReactNode;
  /** Forwarded to the button; an auto id is generated when omitted. */
  id?: string;
  /**
   * Names the control from text that lives elsewhere on the row.
   *
   * `label` renders its text beside the knob, which is right for an inline
   * toggle and wrong for a settings row whose label and hint sit at the other
   * end of a justified line. Without one of the two the control announces as
   * an unnamed switch.
   */
  "aria-labelledby"?: string;
}

export function Switch({
  checked,
  onChange,
  disabled = false,
  busy = false,
  label,
  id,
  "aria-labelledby": ariaLabelledBy,
}: SwitchProps) {
  const autoId = React.useId();
  const buttonId = id ?? autoId;
  const isDisabled = disabled || busy;

  const toggle = () => {
    if (isDisabled) return;
    onChange(!checked);
  };

  const button = (
    <button
      id={buttonId}
      type="button"
      role="switch"
      aria-checked={checked}
      aria-busy={busy || undefined}
      aria-labelledby={ariaLabelledBy}
      disabled={isDisabled}
      onClick={toggle}
      className={cn(
        "xf-switch touch-target--halo press relative inline-flex h-5 w-9 shrink-0 items-center rounded-pill",
        "transition-colors duration-fast ease-soft",
        checked ? "bg-accent" : "bg-active",
        isDisabled ? "cursor-not-allowed opacity-50" : "cursor-pointer",
      )}
    >
      <span
        aria-hidden
        className={cn(
          "pointer-events-none absolute left-0.5 h-4 w-4 rounded-full bg-card shadow-e1",
          "transition-transform duration-fast ease-spring",
          checked ? "translate-x-4" : "translate-x-0",
        )}
      />
    </button>
  );

  if (label == null) return button;

  return (
    <label
      htmlFor={buttonId}
      className={cn(
        "inline-flex items-center gap-2 text-sm text-foreground select-none",
        isDisabled ? "cursor-not-allowed opacity-50" : "cursor-pointer",
      )}
    >
      {button}
      <span>{label}</span>
    </label>
  );
}
