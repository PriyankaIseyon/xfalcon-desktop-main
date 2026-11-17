import React from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "../lib/cn";
import { controlBase, controlSizes } from "./controlStyles";
import { Menu } from "./Menu";

// ---------------------------------------------------------------------------
// Select — a Field-styled control built on top of <Menu>. Replaces the native
// <select>: the trigger shows the active option's label (or a faint
// placeholder) and a trailing chevron; the popover lists the options with a
// check on the active one.
// ---------------------------------------------------------------------------

export interface SelectOption<T extends string = string> {
  value: T;
  label: React.ReactNode;
  icon?: React.ReactNode;
  /**
   * The heading this option sits under. A run of adjacent options with the same
   * `group` is headed once, and a separator sets each run off from the one
   * before it. A list with no groups renders with no headings.
   */
  group?: string;
}

export interface SelectProps<T extends string = string> {
  value: T | undefined;
  onChange: (value: T) => void;
  options: SelectOption<T>[];
  placeholder?: string;
  className?: string;
  disabled?: boolean;
  /**
   * The three things `Field` injects into the control it wraps. Without them a
   * select in a field was a control its own label could not point at: the
   * label carried a `for` and nothing in the document had that id.
   */
  id?: string;
  required?: boolean;
  "aria-invalid"?: boolean;
  "aria-labelledby"?: string;
  "aria-describedby"?: string;
}

export function Select<T extends string = string>({
  value,
  onChange,
  options,
  placeholder = "Select…",
  className,
  disabled = false,
  id,
  required,
  "aria-invalid": invalid,
  "aria-labelledby": labelledBy,
  "aria-describedby": describedBy,
}: SelectProps<T>) {
  const selected = options.find((option) => option.value === value);

  const trigger = (
    <button
      type="button"
      id={id}
      disabled={disabled}
      aria-invalid={invalid}
      aria-labelledby={labelledBy}
      aria-describedby={describedBy}
      // A button cannot carry the native constraint, so the state is said the
      // one way assistive tech can read from a widget.
      aria-required={required || undefined}
      // The field surface, not a copy of it. The copy had drifted three ways:
      // no token duration or curve, so its colour transition ran on Tailwind's
      // default; no `aria-invalid` treatment, so a select could never show
      // itself invalid beside an input that could; and `pointer-events-none`
      // where every other control uses `cursor-not-allowed`, so a disabled
      // select was the one control that gave the cursor no answer.
      className={cn(
        controlBase,
        controlSizes.md,
        "press control-glyph-default inline-flex items-center gap-2",
        className,
      )}
    >
      {selected?.icon ? <span className="flex shrink-0 items-center text-secondary">{selected.icon}</span> : null}
      <span className={cn("flex-1 truncate text-left", selected ? "text-foreground" : "text-faint")}>
        {selected ? selected.label : placeholder}
      </span>
      <ChevronDown className="shrink-0 text-muted" aria-hidden />
    </button>
  );

  return (
    <Menu align="start" rootClassName="w-full" trigger={trigger}>
      {options.map((option, index) => {
        const startsGroup = option.group !== undefined && option.group !== options[index - 1]?.group;
        return (
          <React.Fragment key={option.value}>
            {startsGroup && index > 0 ? <Menu.Separator /> : null}
            {startsGroup ? <Menu.Label>{option.group}</Menu.Label> : null}
            <Menu.Item icon={option.icon} selected={option.value === value} onSelect={() => onChange(option.value)}>
              {option.label}
            </Menu.Item>
          </React.Fragment>
        );
      })}
    </Menu>
  );
}
