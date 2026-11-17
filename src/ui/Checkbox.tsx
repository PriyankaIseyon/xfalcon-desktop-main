import React from "react";
import { Check, Minus } from "lucide-react";
import { cn } from "../lib/cn";

export interface CheckboxProps extends Omit<React.InputHTMLAttributes<HTMLInputElement>, "type"> {
  indeterminate?: boolean;
}

export const Checkbox = React.forwardRef<HTMLInputElement, CheckboxProps>(
  ({ className, indeterminate, checked, disabled, ...props }, ref) => {
    const innerRef = React.useRef<HTMLInputElement | null>(null);
    React.useImperativeHandle(ref, () => innerRef.current as HTMLInputElement);

    React.useEffect(() => {
      if (innerRef.current) innerRef.current.indeterminate = Boolean(indeterminate);
    }, [indeterminate]);

    const isOn = indeterminate || checked;

    return (
      <span className={cn("relative inline-flex size-4 shrink-0", className)}>
        <input
          ref={innerRef}
          type="checkbox"
          checked={checked}
          disabled={disabled}
          className={cn(
            "peer touch-target--halo size-4 shrink-0 cursor-pointer appearance-none rounded-sm border shadow-e1",
            "border-default bg-card transition-colors duration-fast ease-soft",
            "hover:border-strong hover:bg-hover disabled:cursor-not-allowed disabled:opacity-50",
            isOn && "border-accent bg-accent",
          )}
          {...props}
        />
        <span
          aria-hidden
          className={cn(
            "pointer-events-none absolute inset-0 flex size-4 items-center justify-center text-accent-ink",
            "peer-disabled:opacity-50",
          )}
        >
          {indeterminate ? (
            <Minus aria-hidden size={12} strokeWidth={2.2} />
          ) : checked ? (
            <Check aria-hidden size={12} strokeWidth={2.2} />
          ) : null}
        </span>
      </span>
    );
  },
);
Checkbox.displayName = "Checkbox";
