import React from "react";
import ReactDOM from "react-dom";
import { cn } from "../lib/cn";
import { MenuContext, moveMenuFocus, type ContextMenuAnchor } from "./menuContext";
import { useDismissLayer } from "./dismissLayers";
import { useExitAnimation } from "./useExitAnimation";

interface ContextMenuBaseProps {
  /** The anchor, or `null` when closed. */
  anchor: ContextMenuAnchor | null;
  onClose: () => void;
  children: React.ReactNode;
  id?: string;
  className?: string;
}

type ContextMenuName =
  | { /** Names the menu directly — "Actions for Quarterly model". */ label: string; labelledBy?: never }
  | { /** Connects a trigger-anchored menu to the trigger that names it. */ label?: never; labelledBy: string };

export type ContextMenuProps = ContextMenuBaseProps & ContextMenuName;

interface MenuPosition {
  left: number;
  top: number;
  minWidth?: number;
  /** Where the panel grows from: the pointer, or the trigger edge it opened off. */
  origin: string;
}

/** A point inside the panel, in its own pixels, clamped to its box. */
function originAt(x: number, y: number, width: number, height: number): string {
  return `${Math.max(0, Math.min(x, width))}px ${Math.max(0, Math.min(y, height))}px`;
}

/** Keeps either menu placement inside the window when its anchor is near an edge. */
function clampToViewport(anchor: ContextMenuAnchor, panel: HTMLElement, host: HTMLElement | null): MenuPosition {
  const { offsetWidth, offsetHeight } = panel;
  const margin = 8;
  const gap = 4;
  const hostRect = host?.getBoundingClientRect();
  const trigger = anchor.trigger;
  if (trigger) {
    const panelWidth = Math.max(offsetWidth, trigger.width);
    let side = trigger.side;
    if (side === "bottom" && trigger.bottom + gap + offsetHeight > window.innerHeight - margin) side = "top";
    if (side === "top" && trigger.top - gap - offsetHeight < margin) side = "bottom";
    const desiredLeft = trigger.align === "start" ? trigger.left : trigger.right - panelWidth;
    const desiredTop = side === "bottom" ? trigger.bottom + gap : trigger.top - offsetHeight - gap;
    const left = Math.max(margin, Math.min(desiredLeft, window.innerWidth - panelWidth - margin));
    const top = Math.max(margin, Math.min(desiredTop, window.innerHeight - offsetHeight - margin));
    return {
      left: left - (hostRect?.left ?? 0),
      top: top - (hostRect?.top ?? 0),
      // `panelWidth`, not `trigger.width`. The panel is measured under the
      // class floor (`min-w-48`) and placed for that width, but an inline
      // `min-width` OVERRIDES the class -- so pinning the trigger's own width
      // let the panel re-lay-out narrower than the left edge was solved for,
      // and an end-aligned menu came to rest short of its trigger.
      minWidth: panelWidth,
      // The side it actually opened on: a menu flipped above its trigger
      // grows upward out of it, not down toward it.
      origin: originAt(
        (trigger.align === "start" ? trigger.left : trigger.right) - left,
        side === "bottom" ? 0 : offsetHeight,
        panelWidth,
        offsetHeight,
      ),
    };
  }
  const left = Math.max(margin, Math.min(anchor.x, window.innerWidth - offsetWidth - margin));
  const top = Math.max(margin, Math.min(anchor.y, window.innerHeight - offsetHeight - margin));
  return {
    left: left - (hostRect?.left ?? 0),
    top: top - (hostRect?.top ?? 0),
    origin: originAt(anchor.x - left, anchor.y - top, offsetWidth, offsetHeight),
  };
}

/**
 * The shared portalled surface for pointer and trigger menus. Both presenters
 * therefore use the same item registry, keyboard movement, viewport clamp,
 * focus return, and modal focus-scope behavior.
 *
 * Every row action in the sidebar used to exist ONLY as a button that appeared
 * on hover. That made deleting a chat or removing a project undiscoverable
 * (you had to hover the right row to learn it was possible), unreachable
 * without a pointer, and invisible on a touch screen. Right-click is where
 * people look for what a row can do, so that is where the full set lives now;
 * the hover buttons stay as the shortcut for the one or two most-used ones.
 *
 * Rendered in a portal so a row inside an `overflow: hidden` band — which is
 * every row in the sidebar — cannot clip its own menu.
 */
export function ContextMenu({ anchor, onClose, label, labelledBy, children, id, className }: ContextMenuProps) {
  const panelRef = React.useRef<HTMLDivElement | null>(null);
  const itemsRef = React.useRef<HTMLButtonElement[]>([]);
  const [position, setPosition] = React.useState<MenuPosition | null>(null);
  const exitRef = useExitAnimation(panelRef);
  const portalHost = anchor?.returnFocus?.closest<HTMLElement>('[role="dialog"], [role="alertdialog"]') ?? null;

  const registerItem = React.useCallback((node: HTMLButtonElement | null, disabled: boolean) => {
    const items = itemsRef.current;
    const existing = node ? items.indexOf(node) : -1;
    if (!node || disabled) {
      if (existing !== -1) items.splice(existing, 1);
      return;
    }
    if (existing === -1) items.push(node);
    items.sort((a, b) => (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1));
  }, []);

  const close = React.useCallback(() => {
    itemsRef.current = [];
    anchor?.returnFocus?.focus();
    onClose();
  }, [anchor, onClose]);

  // Measure after paint, so the clamp works against the panel's real size
  // rather than a guess. Held as state (not a layout write) so the first
  // painted frame is already in the right place.
  React.useLayoutEffect(() => {
    if (anchor == null) {
      setPosition(null);
      return;
    }
    const panel = panelRef.current;
    if (panel) setPosition(clampToViewport(anchor, panel, portalHost));
  }, [anchor, portalHost]);

  React.useEffect(() => {
    if (anchor == null) return;
    const selected = itemsRef.current.find((item) => item.dataset.selected === "true");
    const target =
      anchor.initialFocus === "last"
        ? itemsRef.current.at(-1)
        : anchor.initialFocus === "selected"
          ? (selected ?? itemsRef.current[0])
          : itemsRef.current[0];
    target?.focus();
  }, [anchor, position]);

  // Escape and Tab belong to the topmost open layer, which is this menu for as
  // long as it is open -- including when it was opened from inside a dialog,
  // whose own trap would otherwise answer Escape first and take the dialog with
  // it. See `dismissLayers.ts`.
  //
  // Tab CLOSES the menu and hands focus back to the trigger rather than letting
  // the focused item unmount underneath the caret: the browser reads focus
  // after dispatch, so returning it first makes Tab land on whatever follows
  // the trigger instead of restarting from the top of the document.
  useDismissLayer(anchor != null, { onEscape: close, onTab: close });

  React.useEffect(() => {
    if (anchor == null) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!panelRef.current?.contains(target) && !anchor.returnFocus?.contains(target)) close();
    };
    // An open menu does not track layout continuously, so it closes rather
    // than hanging over unrelated content once the viewport moves underneath.
    const onScroll = (event: Event) => {
      // Scrolling a long menu must not dismiss the menu itself.
      if (event.target instanceof Node && panelRef.current?.contains(event.target)) return;
      close();
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onScroll);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onScroll);
    };
  }, [anchor, close]);

  if (anchor == null) return null;

  const context = { close, registerItem };

  return ReactDOM.createPortal(
    <MenuContext.Provider value={context}>
      <div
        ref={exitRef}
        id={id}
        role="menu"
        aria-label={label}
        aria-labelledby={labelledBy}
        aria-orientation="vertical"
        tabIndex={-1}
        onKeyDown={(event) => {
          if (moveMenuFocus(itemsRef.current, event.key)) event.preventDefault();
        }}
        className={cn(
          "xf-material xf-overlay-pop z-[var(--xf-z-dropdown)] max-h-[calc(100dvh-1rem)] overflow-y-auto min-w-48 rounded-lg border border-default p-1 shadow-e3",
          portalHost ? "absolute" : "fixed",
          className,
        )}
        style={{
          left: position?.left ?? anchor.x,
          top: position?.top ?? anchor.y,
          minWidth: position?.minWidth,
          transformOrigin: position?.origin,
          // Invisible for the one frame between mount and measurement, so the
          // panel is never seen at the unclamped position.
          visibility: position == null ? "hidden" : "visible",
        }}
      >
        {children}
      </div>
    </MenuContext.Provider>,
    portalHost ?? document.body,
  );
}
