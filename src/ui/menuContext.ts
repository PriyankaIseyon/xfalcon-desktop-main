import React from "react";

/**
 * What a menu item needs from whatever is presenting it: how to dismiss, and
 * how to enrol itself in the arrow-key order.
 *
 * Its own module because two surfaces present the same items now — `Menu`,
 * anchored under a trigger, and `ContextMenu`, anchored at the pointer — and
 * both need this context without either importing the other. Keeping it in
 * `Menu.tsx` would also cost that file its Fast Refresh, the same way
 * `useModalFocusTrap` did before it moved out of `Modal.tsx`.
 */
/** Where a pointer or trigger menu was summoned, in viewport coordinates. */
export interface ContextMenuAnchor {
  x: number;
  y: number;
  /** The row that owns the menu, for Escape and command focus return. */
  returnFocus?: HTMLElement | null;
  /** Trigger geometry for a menu anchored to a control rather than a pointer. */
  trigger?: {
    left: number;
    right: number;
    top: number;
    bottom: number;
    width: number;
    align: "start" | "end";
    side: "top" | "bottom";
  };
  initialFocus?: "first" | "last" | "selected";
}

/**
 * Turns a pointer or keyboard event into an anchor.
 *
 * The keyboard cases matter as much as the pointer one: Shift+F10 and the Menu
 * key are how a keyboard user asks "what can this row do", and they carry no
 * coordinates, so the row's own corner stands in.
 */
export function contextMenuAnchorFrom(
  event: React.MouseEvent | React.KeyboardEvent,
  fallback: HTMLElement | null,
): ContextMenuAnchor {
  if ("clientX" in event && event.clientX !== 0) {
    return { x: event.clientX, y: event.clientY, returnFocus: fallback };
  }
  const rect = fallback?.getBoundingClientRect();
  return { x: rect ? rect.left + 12 : 0, y: rect ? rect.bottom - 4 : 0, returnFocus: fallback };
}

/** True for the two keystrokes that mean "open this row's context menu". */
export function isContextMenuKey(event: React.KeyboardEvent): boolean {
  return event.key === "ContextMenu" || (event.shiftKey && event.key === "F10");
}

export interface MenuContextValue {
  close: () => void;
  registerItem: (node: HTMLButtonElement | null, disabled: boolean) => void;
}

export const MenuContext = React.createContext<MenuContextValue | null>(null);

export function useMenuContext(component: string): MenuContextValue {
  const context = React.useContext(MenuContext);
  if (!context) throw new Error(`<Menu.${component}> must be rendered inside <Menu> or <ContextMenu>.`);
  return context;
}

/**
 * Roving focus shared by both presentations: Up/Down wrap, Home/End jump.
 * Returns whether the key was handled, so a caller can leave everything else
 * to bubble.
 */
export function moveMenuFocus(items: readonly HTMLButtonElement[], key: string): boolean {
  if (items.length === 0) return false;
  const current = items.indexOf(document.activeElement as HTMLButtonElement);
  const focusAt = (index: number) => items[(index + items.length) % items.length]?.focus();
  switch (key) {
    case "ArrowDown":
      focusAt(current + 1);
      return true;
    case "ArrowUp":
      focusAt(current - 1);
      return true;
    case "Home":
      focusAt(0);
      return true;
    case "End":
      focusAt(items.length - 1);
      return true;
    default:
      if (key.length !== 1 || /\s/.test(key)) return false;
      for (let offset = 1; offset <= items.length; offset += 1) {
        const candidate = items[(current + offset + items.length) % items.length];
        if (candidate?.textContent?.trim().toLocaleLowerCase().startsWith(key.toLocaleLowerCase())) {
          candidate.focus();
          return true;
        }
      }
      return false;
  }
}
