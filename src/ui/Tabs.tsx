import React from "react";
import { cn } from "../lib/cn";
import { useElasticSelection } from "./useElasticSelection";

export interface TabItem {
  id: string;
  label: React.ReactNode;
  disabled?: boolean;
}

type AccessibleTablistName =
  | { "aria-label": string; "aria-labelledby"?: never }
  | { "aria-label"?: never; "aria-labelledby": string };

export type TabsProps = {
  tabs: TabItem[];
  value: string;
  onChange: (id: string) => void;
  className?: string;
  panelClassName?: string;
  children: React.ReactNode;
} & AccessibleTablistName;

export function Tabs({
  tabs,
  value,
  onChange,
  className,
  panelClassName,
  children,
  "aria-label": ariaLabel,
  "aria-labelledby": ariaLabelledBy,
}: TabsProps) {
  const tabRefs = React.useRef<(HTMLButtonElement | null)[]>([]);
  const stripRef = React.useRef<HTMLDivElement>(null);
  const instanceId = React.useId();
  const activeIndex = tabs.findIndex((tab) => tab.id === value && !tab.disabled);

  function selectByIndex(index: number) {
    const tab = tabs[index];
    if (!tab || tab.disabled) return;
    onChange(tab.id);
    tabRefs.current[index]?.focus();
  }

  const elastic = useElasticSelection({
    containerRef: stripRef,
    itemRefs: tabRefs,
    selectedIndex: activeIndex,
    enabledIndexes: tabs.flatMap((tab, index) => tab.disabled ? [] : [index]),
    itemKey: JSON.stringify(tabs.map((tab) => tab.id)),
    onSelect: selectByIndex,
  });

  function handleKeyDown(e: React.KeyboardEvent<HTMLDivElement>) {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) return;
    const enabledIndexes = tabs.flatMap((tab, index) => (tab.disabled ? [] : [index]));
    if (enabledIndexes.length === 0) return;
    e.preventDefault();
    const current = enabledIndexes.indexOf(activeIndex);
    const from = current === -1 ? 0 : current;
    const nextEnabled =
      e.key === "Home"
        ? 0
        : e.key === "End"
          ? enabledIndexes.length - 1
          : (from + (e.key === "ArrowRight" ? 1 : -1) + enabledIndexes.length) % enabledIndexes.length;
    const next = enabledIndexes[nextEnabled];
    if (next != null) selectByIndex(next);
  }

  const firstEnabledIndex = tabs.findIndex((tab) => !tab.disabled);
  const selectedIndex = activeIndex;
  const focusIndex = activeIndex === -1 ? firstEnabledIndex : activeIndex;
  const selectedTabId = selectedIndex === -1 ? undefined : `${instanceId}-tab-${selectedIndex}`;
  // ONE panel id, not one per tab. `children` is the body of whichever tab is
  // current, so only one panel is ever in the document; a per-tab id gave every
  // unselected tab an `aria-controls` pointing at nothing, which is a dangling
  // IDREF rather than a relationship. Only the selected tab controls anything.
  const panelId = `${instanceId}-panel`;

  return (
    <>
      <div
        ref={stripRef}
        role="tablist"
        aria-label={ariaLabel}
        aria-labelledby={ariaLabelledBy}
        aria-orientation="horizontal"
        onKeyDown={handleKeyDown}
        onPointerDown={elastic.onPointerDown}
        onPointerMove={elastic.onPointerMove}
        onPointerUp={elastic.onPointerUp}
        onPointerCancel={elastic.onPointerCancel}
        onLostPointerCapture={elastic.onLostPointerCapture}
        className={cn("tabs-strip elastic-selection", className)}
      >
        <span ref={elastic.thumbRef} aria-hidden="true" data-elastic-thumb className="elastic-selection__thumb" />
        {tabs.map((tab, index) => {
          const isActive = index === selectedIndex;
          return (
            <button
              key={tab.id}
              ref={(el) => {
                tabRefs.current[index] = el;
              }}
              id={`${instanceId}-tab-${index}`}
              type="button"
              role="tab"
              aria-controls={isActive ? panelId : undefined}
              aria-selected={isActive}
              disabled={tab.disabled}
              tabIndex={index === focusIndex ? 0 : -1}
              onClick={(event) => {
                if (elastic.consumeClick(event)) return;
                selectByIndex(index);
              }}
              className={cn(
                "tabs-strip__tab elastic-selection__item press touch-target",
                tab.disabled
                  ? "cursor-not-allowed text-faint opacity-50"
                  : isActive
                    ? "cursor-pointer text-foreground"
                    : "cursor-pointer text-muted hover:text-foreground",
              )}
            >
              {tab.label}
            </button>
          );
        })}
      </div>
      <div
        id={panelId}
        role="tabpanel"
        aria-labelledby={selectedTabId}
        tabIndex={0}
        className={panelClassName}
      >
        {children}
      </div>
    </>
  );
}
