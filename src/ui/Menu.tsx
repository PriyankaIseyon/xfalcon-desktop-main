import React from "react";
import { Check } from "lucide-react";
import { cn } from "../lib/cn";
import { ContextMenu } from "./ContextMenu";
import { type ContextMenuAnchor, useMenuContext } from "./menuContext";
import { Tooltip, type TooltipSide } from "./Tooltip";

type Align = "start" | "end";
type Side = "top" | "bottom";

export interface MenuProps {
  trigger: React.ReactElement;
  /** Help for an icon-only trigger. The menu owns the trigger ref, so the
   * shared tooltip wraps the already-injected element here. */
  triggerTooltip?: { label: React.ReactNode; side?: TooltipSide };
  align?: Align;
  side?: Side;
  children: React.ReactNode;
  className?: string;
  /** Class for the trigger root. A field menu must fill the field width. */
  rootClassName?: string;
}

interface TriggerInjectedProps {
  ref: React.Ref<HTMLElement>;
  id: string;
  onClick: (event: React.MouseEvent) => void;
  onKeyDown: (event: React.KeyboardEvent) => void;
  "aria-haspopup": "menu";
  "aria-expanded": boolean;
  "aria-controls": string | undefined;
}

export function Menu({
  trigger,
  triggerTooltip,
  align = "start",
  side = "bottom",
  children,
  className,
  rootClassName,
}: MenuProps) {
  const [anchor, setAnchor] = React.useState<ContextMenuAnchor | null>(null);
  const triggerRef = React.useRef<HTMLElement | null>(null);
  const generatedId = React.useId();
  const triggerId = (trigger.props as { id?: string }).id ?? `${generatedId}-trigger`;
  const panelId = `${generatedId}-panel`;

  const openMenu = React.useCallback(
    (initialFocus: "first" | "last" | "selected") => {
      const triggerElement = triggerRef.current;
      if (!triggerElement) return;
      const rect = triggerElement.getBoundingClientRect();
      setAnchor({
        x: align === "start" ? rect.left : rect.right,
        y: side === "bottom" ? rect.bottom : rect.top,
        returnFocus: triggerElement,
        initialFocus,
        trigger: {
          left: rect.left,
          right: rect.right,
          top: rect.top,
          bottom: rect.bottom,
          width: rect.width,
          align,
          side,
        },
      });
    },
    [align, side],
  );

  const injected: TriggerInjectedProps = {
    ref: (node: HTMLElement | null) => {
      triggerRef.current = node;
      const original = (trigger as { ref?: React.Ref<HTMLElement> }).ref;
      if (typeof original === "function") original(node);
      else if (original && typeof original === "object") original.current = node;
    },
    id: triggerId,
    onClick: (event) => {
      event.stopPropagation();
      const original = (trigger.props as { onClick?: (clickEvent: React.MouseEvent) => void }).onClick;
      original?.(event);
      if (event.defaultPrevented) return;
      if (anchor) setAnchor(null);
      else openMenu("selected");
    },
    onKeyDown: (event) => {
      const original = (trigger.props as { onKeyDown?: (keyEvent: React.KeyboardEvent) => void }).onKeyDown;
      original?.(event);
      if (event.defaultPrevented || (event.key !== "ArrowDown" && event.key !== "ArrowUp")) return;
      event.preventDefault();
      openMenu(event.key === "ArrowUp" ? "last" : "first");
    },
    "aria-haspopup": "menu",
    "aria-expanded": anchor != null,
    "aria-controls": anchor ? panelId : undefined,
  };

  const injectedTrigger = React.cloneElement(trigger, injected);
  const renderedTrigger = triggerTooltip ? (
    <Tooltip label={triggerTooltip.label} side={triggerTooltip.side ?? "top"}>
      {injectedTrigger}
    </Tooltip>
  ) : (
    injectedTrigger
  );

  return (
    <span className={cn("relative inline-flex", rootClassName)}>
      {renderedTrigger}
      <ContextMenu
        anchor={anchor}
        className={className}
        id={panelId}
        labelledBy={triggerId}
        onClose={() => setAnchor(null)}
      >
        {children}
      </ContextMenu>
    </span>
  );
}

export interface MenuItemProps {
  children: React.ReactNode;
  icon?: React.ReactNode;
  onSelect?: () => void;
  disabled?: boolean;
  selected?: boolean;
  variant?: "default" | "danger";
  className?: string;
}

function MenuItem({
  children,
  icon,
  onSelect,
  disabled = false,
  selected,
  variant = "default",
  className,
}: MenuItemProps) {
  const { close, registerItem } = useMenuContext("Item");
  const ref = React.useRef<HTMLButtonElement | null>(null);

  React.useEffect(() => {
    const element = ref.current;
    registerItem(element, disabled);
    return () => registerItem(element, true);
  }, [disabled, registerItem]);

  const select = () => {
    if (disabled) return;
    onSelect?.();
    close();
  };

  return (
    <button
      ref={ref}
      type="button"
      role={selected === undefined ? "menuitem" : "menuitemradio"}
      tabIndex={-1}
      aria-disabled={disabled}
      aria-checked={selected}
      data-selected={selected || undefined}
      data-variant={variant}
      disabled={disabled}
      onClick={select}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          select();
        }
      }}
      className={cn(
        "flex min-h-control-compact w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-sm transition-colors duration-fast ease-soft",
        variant === "danger" ? "text-danger focus:text-danger" : "text-foreground",
        disabled ? "cursor-default text-faint" : "cursor-pointer hover:bg-hover focus:bg-hover",
        className,
      )}
    >
      <span className="flex w-4 shrink-0 items-center justify-center text-secondary">{icon}</span>
      <span className="flex-1 truncate">{children}</span>
      <span className="flex w-4 shrink-0 items-center justify-center text-accent">
        {selected ? <Check size={14} strokeWidth={2.2} aria-hidden /> : null}
      </span>
    </button>
  );
}

export interface MenuSeparatorProps {
  className?: string;
}

function MenuSeparator({ className }: MenuSeparatorProps) {
  return <div role="separator" aria-orientation="horizontal" className={cn("my-1 h-px bg-hairline", className)} />;
}

export interface MenuLabelProps {
  children: React.ReactNode;
  className?: string;
}

function MenuLabel({ children, className }: MenuLabelProps) {
  return (
    <div className={cn("px-2.5 pb-1 pt-1.5 text-xs font-medium text-muted select-none", className)}>{children}</div>
  );
}

Menu.Item = MenuItem;
Menu.Separator = MenuSeparator;
Menu.Label = MenuLabel;
