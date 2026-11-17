import React from "react";
import { cn } from "../lib/cn";

export interface RadioGroupProps {
  name: string;
  value: string;
  onValueChange: (value: string) => void;
  label?: string;
  className?: string;
  children: React.ReactNode;
}

export function RadioGroup({ name, value, onValueChange, label, className, children }: RadioGroupProps) {
  return (
    <div role="radiogroup" aria-label={label} className={cn("flex flex-col gap-2", className)}>
      {React.Children.map(children, (child) => {
        if (!React.isValidElement<RadioProps>(child)) return child;
        return React.cloneElement(child, {
          name,
          checked: child.props.value === value,
          onValueChange,
        });
      })}
    </div>
  );
}

export interface RadioProps {
  value: string;
  name?: string;
  checked?: boolean;
  disabled?: boolean;
  onValueChange?: (value: string) => void;
  children?: React.ReactNode;
  className?: string;
}

export const Radio = React.forwardRef<HTMLInputElement, RadioProps>(
  ({ className, value, checked, disabled, onValueChange, children, name }, ref) => {
    return (
      <label
        className={cn(
          "inline-flex cursor-pointer items-center gap-2 text-sm text-foreground select-none",
          // The one place a disabled radio dims. The input carried its own
          // `disabled:opacity-50`, which compounded with this to render the
          // ring at a quarter opacity beside a dot at half.
          disabled && "cursor-not-allowed opacity-50",
          className,
        )}
      >
        <span className="relative inline-flex size-4 shrink-0">
          <input
            ref={ref}
            type="radio"
            name={name}
            value={value}
            checked={checked}
            disabled={disabled}
            className={cn(
              "peer touch-target--halo size-4 shrink-0 cursor-pointer appearance-none rounded-full border bg-card",
              "border-default transition-colors duration-fast ease-soft hover:border-strong",
              "disabled:cursor-not-allowed",
              checked && "border-accent",
            )}
            onChange={() => onValueChange?.(value)}
          />
          <span aria-hidden className="pointer-events-none absolute inset-0 flex size-4 items-center justify-center">
            <span
              className={cn(
                // No `peer-disabled:` here: this dot is a GRANDCHILD of the
                // input, and `peer-*` compiles to a sibling combinator, so the
                // utility matched nothing. The label above dims the whole
                // control, dot included.
                "size-2 rounded-full bg-accent transition-transform duration-fast ease-spring",
                checked ? "scale-100" : "scale-0",
              )}
            />
          </span>
        </span>
        {children ? <span className="text-secondary">{children}</span> : null}
      </label>
    );
  },
);
Radio.displayName = "Radio";
