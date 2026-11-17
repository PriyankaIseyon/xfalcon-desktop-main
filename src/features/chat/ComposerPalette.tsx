import React, { useEffect, useRef } from "react";
import { cn } from "../../lib/cn";
import { Shortcut } from "../../ui/Kbd";
import { useExitAnimation } from "../../ui/useExitAnimation";
import type { PaletteItem, PaletteKind } from "./composerDefaults";

export interface ComposerPaletteProps {
  kind: PaletteKind;
  query: string;
  items: PaletteItem[];
  selectedIndex: number;
  onSelect: (item: PaletteItem) => void;
  onClose: () => void;
}

export function ComposerPalette({ kind, query, items, selectedIndex, onSelect, onClose }: ComposerPaletteProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const listRef = useRef<HTMLUListElement | null>(null);
  const exitRef = useExitAnimation(containerRef);

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [onClose]);

  useEffect(() => {
    const activeEl = listRef.current?.querySelector<HTMLElement>(`[data-index="${selectedIndex}"]`);
    activeEl?.scrollIntoView?.({ block: "nearest" });
  }, [selectedIndex]);

  return (
    <div
      ref={exitRef}
      id="composer-palette-list"
      aria-label={kind === "mention" ? "Mention suggestions" : "Slash commands"}
      className="xf-overlay-pop absolute bottom-full left-0 z-30 mb-2 origin-bottom-left w-full max-w-md overflow-hidden rounded-xl border border-default bg-card/95 shadow-e3 backdrop-blur-md"
      role="listbox"
    >
      <div className="composer-palette__header flex flex-wrap items-center justify-between gap-x-3 gap-y-1 border-b border-default/60 px-3 py-1.5 text-2xs font-medium uppercase tracking-wider text-muted">
        <span className="shrink-0">{kind === "mention" ? "Reference Context (@)" : "Slash Commands (/)"}</span>
        <span className="composer-palette__shortcuts ml-auto flex flex-wrap items-center justify-end gap-x-1 gap-y-0.5 text-right">
          <Shortcut keys={["Up"]} />
          <Shortcut keys={["Down"]} /> navigate <Shortcut keys={["Enter"]} /> select <Shortcut keys={["Escape"]} />{" "}
          dismiss
        </span>
      </div>
      <ul ref={listRef} className="composer-palette__results overflow-y-auto p-1.5" role="presentation">
        {items.map((item, index) => {
          const isSelected = index === selectedIndex;
          return (
            <li
              key={item.id}
              id={`composer-palette-item-${item.id}`}
              aria-selected={isSelected}
              data-index={index}
              className={cn(
                "flex cursor-pointer items-center justify-between gap-3 rounded-lg px-2.5 py-2 text-sm transition-colors duration-fast",
                isSelected
                  ? "bg-active text-foreground font-medium"
                  : "text-secondary hover:bg-hover hover:text-foreground",
              )}
              onClick={() => onSelect(item)}
              role="option"
            >
              <div className="flex items-center gap-2.5 min-w-0">
                <span className={cn("shrink-0 text-muted", isSelected && "text-accent")}>{item.icon}</span>
                <div className="truncate">
                  <span className="text-foreground">{item.title}</span>
                  {item.subtitle ? <span className="ml-2 text-xs text-muted truncate">{item.subtitle}</span> : null}
                </div>
              </div>
              <span className="shrink-0 text-2xs uppercase tracking-wider text-muted/80">{item.category}</span>
            </li>
          );
        })}
        {items.length === 0 ? (
          <li role="presentation" className="p-3 type-caption text-secondary">
            <p role="status">
              No matching {kind === "mention" ? "files or resources" : "commands"} found for &ldquo;{query}&rdquo;
            </p>
          </li>
        ) : null}
      </ul>
    </div>
  );
}
