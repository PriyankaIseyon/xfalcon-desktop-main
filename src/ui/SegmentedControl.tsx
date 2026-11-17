import React from "react";
import { cn } from "../lib/cn";
import { useElasticSelection } from "./useElasticSelection";

export interface SegmentOption<T extends string = string> {
  value: T;
  label: React.ReactNode;
  icon?: React.ReactNode;
  disabled?: boolean;
}

type AccessibleRadiogroupName =
  | { "aria-label": string; "aria-labelledby"?: never }
  | { "aria-label"?: never; "aria-labelledby": string };

export type SegmentedControlProps<T extends string = string> = {
  options: SegmentOption<T>[];
  value: T;
  onChange: (value: T) => void;
  className?: string;
  /**
   * `solid` is the app's inset track. `settings` is the explicit Settings-only
   * treatment. The former pill variant had no caller and broke the product
   * rule that reserves pills for chips, status, and circular actions.
   */
  variant?: "solid" | "settings";
} & AccessibleRadiogroupName;

export function SegmentedControl<T extends string = string>({
  options,
  value,
  onChange,
  className,
  variant = "solid",
  "aria-label": ariaLabel,
  "aria-labelledby": ariaLabelledBy,
}: SegmentedControlProps<T>) {
  const isSettings = variant === "settings";
  const activeIndex = options.findIndex((option) => option.value === value && !option.disabled);
  const optionRefs = React.useRef<Array<HTMLButtonElement | null>>([]);
  const groupRef = React.useRef<HTMLDivElement>(null);

  const selectByIndex = (index: number) => {
    const option = options[index];
    if (!option || option.disabled) return;
    onChange(option.value);
    optionRefs.current[index]?.focus();
  };

  const elastic = useElasticSelection({
    containerRef: groupRef,
    itemRefs: optionRefs,
    selectedIndex: activeIndex,
    enabledIndexes: options.flatMap((option, index) => option.disabled ? [] : [index]),
    itemKey: JSON.stringify(options.map((option) => option.value)),
    onSelect: selectByIndex,
  });

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    const enabledIndexes = options.flatMap((option, index) => (option.disabled ? [] : [index]));
    if (enabledIndexes.length === 0) return;
    event.preventDefault();
    const currentEnabledIndex = enabledIndexes.indexOf(activeIndex);
    const from = currentEnabledIndex === -1 ? 0 : currentEnabledIndex;
    const nextEnabledIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? enabledIndexes.length - 1
          : (from + (event.key === "ArrowRight" ? 1 : -1) + enabledIndexes.length) % enabledIndexes.length;
    const nextIndex = enabledIndexes[nextEnabledIndex];
    if (nextIndex == null) return;
    selectByIndex(nextIndex);
  };

  const firstEnabledIndex = options.findIndex((option) => !option.disabled);

  return (
    <div
      ref={groupRef}
      role="radiogroup"
      aria-orientation="horizontal"
      aria-label={ariaLabel}
      aria-labelledby={ariaLabelledBy}
      onKeyDown={handleKeyDown}
      onPointerDown={elastic.onPointerDown}
      onPointerMove={elastic.onPointerMove}
      onPointerUp={elastic.onPointerUp}
      onPointerCancel={elastic.onPointerCancel}
      onLostPointerCapture={elastic.onLostPointerCapture}
      data-settings-segmented-control={isSettings ? "" : undefined}
      className={cn(
        "elastic-selection inline-flex max-w-full items-center overflow-x-auto",
        isSettings
          ? "settings-segmented-control"
          : "rounded-md border border-default bg-surface-alt p-0.5",
        className,
      )}
    >
      <span ref={elastic.thumbRef} aria-hidden="true" data-elastic-thumb className="elastic-selection__thumb" />
      {options.map((option, index) => {
        const isActive = option.value === value;
        return (
          <button
            key={option.value}
            ref={(node) => {
              optionRefs.current[index] = node;
            }}
            type="button"
            role="radio"
            aria-checked={isActive}
            aria-disabled={option.disabled || undefined}
            data-state={isSettings ? (isActive ? "checked" : "unchecked") : undefined}
            disabled={option.disabled}
            tabIndex={!option.disabled && (isActive || (activeIndex === -1 && index === firstEnabledIndex)) ? 0 : -1}
            onClick={(event) => {
              if (elastic.consumeClick(event)) return;
              selectByIndex(index);
            }}
            className={cn(
              "elastic-selection__item touch-target inline-flex min-h-control-compact items-center justify-center gap-1.5 font-medium select-none",
              option.disabled ? "cursor-not-allowed opacity-50" : "cursor-pointer",
              isSettings
                ? "settings-segmented-control__segment"
                : "press transition-[color,transform] duration-fast ease-soft",
              isSettings
                ? null
                : "rounded-sm px-3 py-1 text-sm",
              !isSettings && isActive && "text-foreground",
              !isSettings && !isActive && "text-muted hover:text-foreground",
            )}
          >
            {option.icon ? (
              <span
                aria-hidden={isSettings ? "true" : undefined}
                className={cn(
                  "flex shrink-0 items-center",
                  isSettings && "settings-segmented-control__icon",
                )}
              >
                {option.icon}
              </span>
            ) : null}
            <span className="truncate">{option.label}</span>
          </button>
        );
      })}
    </div>
  );
}
