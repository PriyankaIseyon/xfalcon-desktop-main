import React from "react";
import { cn } from "../lib/cn";

export interface SliderProps extends Omit<React.InputHTMLAttributes<HTMLInputElement>, "type"> {
  label?: string;
}

export const Slider = React.forwardRef<HTMLInputElement, SliderProps>(
  ({ className, label, id, value, defaultValue, ...props }, ref) => {
    const autoId = React.useId();
    const inputId = id ?? autoId;

    return (
      <div className={cn("flex w-full flex-col gap-2", className)}>
        {label ? (
          <label htmlFor={inputId} className="type-label font-medium text-secondary">
            {label}
          </label>
        ) : null}
        <input
          ref={ref}
          id={inputId}
          type="range"
          value={value}
          defaultValue={defaultValue}
          className={cn(
            "xf-slider touch-target--halo h-2 w-full cursor-pointer appearance-none rounded-pill bg-active",
            "disabled:cursor-not-allowed disabled:opacity-50",
          )}
          {...props}
        />
      </div>
    );
  },
);
Slider.displayName = "Slider";
