import { useEffect, useLayoutEffect, useRef, useState } from "react";

/**
 * Drag handle on the open sidebar's right edge (Kyle, 2026-10-15).
 *
 * Same contract as the artifact panel's divider (`CanvasSlot`): a focusable
 * separator that drags, steps with the arrow keys, jumps with Home and End, and
 * resets with Enter or a double-click. The width lives in `--xf-sidebar-width`
 * on the shell, which every sidebar track already reads.
 *
 * It holds for the session only. Desktop keeps no web storage (docs/security.md,
 * "No browser authority"), so remembering it across launches means a field in
 * the native appearance settings.
 */
export const SIDEBAR_MIN_WIDTH = 240;
export const SIDEBAR_MAX_WIDTH = 480;
export const SIDEBAR_DEFAULT_WIDTH = 304; // 19rem, the token's own value

/** The width chosen this session, so collapsing and reopening keeps it. */
let sessionWidth = SIDEBAR_DEFAULT_WIDTH;

const clamp = (value: number) => Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, Math.round(value)));

export function SidebarResizer() {
  const handleRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; width: number; shell: HTMLElement } | null>(null);
  const [width, setWidth] = useState(() => sessionWidth);

  useLayoutEffect(() => {
    const shell = handleRef.current?.closest<HTMLElement>(".app-shell");
    shell?.style.setProperty("--xf-sidebar-width", `${width}px`);
  }, [width]);

  // Collapsing the sidebar with the handle focused unmounts it, and focus would
  // fall to the body. Hand focus to the title bar's toggle, which stays mounted
  // and is the control that collapsed the sidebar. Layout cleanup runs before the
  // handle leaves the DOM, so the check can still see it focused. The shell
  // outlives this subtree, so it anchors the lookup, and the move waits a
  // microtask until React has removed the handle.
  useLayoutEffect(() => {
    const handle = handleRef.current;
    const shell = handle?.closest<HTMLElement>(".app-shell");
    return () => {
      if (!shell || document.activeElement !== handle) return;
      queueMicrotask(() => {
        shell.querySelector<HTMLElement>('.app-titlebar [aria-controls="app-sidebar"]')?.focus();
      });
    };
  }, []);

  // Collapsing the sidebar mid-drag unmounts the handle: release the shell's
  // drag state, which pointerup can no longer do.
  useEffect(
    () => () => {
      if (drag.current) delete drag.current.shell.dataset.sidebarResizing;
      drag.current = null;
    },
    [],
  );

  const finish = () => {
    if (drag.current) delete drag.current.shell.dataset.sidebarResizing;
    drag.current = null;
  };
  const resize = (next: number) => {
    sessionWidth = clamp(next);
    setWidth(sessionWidth);
  };

  return (
    <div
      ref={handleRef}
      className="sidebar-resizer"
      role="separator"
      aria-label="Resize sidebar"
      aria-controls="app-sidebar"
      aria-description="Drag or use Left and Right arrows to resize. Double-click or press Enter to reset."
      aria-orientation="vertical"
      aria-valuemin={SIDEBAR_MIN_WIDTH}
      aria-valuemax={SIDEBAR_MAX_WIDTH}
      aria-valuenow={width}
      aria-valuetext={`${width} pixels wide`}
      tabIndex={0}
      onDoubleClick={() => resize(SIDEBAR_DEFAULT_WIDTH)}
      onPointerDown={(event) => {
        if (event.button !== 0 || !event.isPrimary) return;
        const shell = handleRef.current?.closest<HTMLElement>(".app-shell");
        if (!shell) return;
        event.preventDefault();
        event.currentTarget.focus();
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { x: event.clientX, width, shell };
        shell.dataset.sidebarResizing = "true";
      }}
      onPointerMove={(event) => {
        const start = drag.current;
        if (!start) return;
        resize(start.width + (event.clientX - start.x));
      }}
      onPointerUp={(event) => {
        finish();
        if (event.currentTarget.hasPointerCapture(event.pointerId))
          event.currentTarget.releasePointerCapture(event.pointerId);
      }}
      onPointerCancel={finish}
      onLostPointerCapture={finish}
      onKeyDown={(event) => {
        const step = event.shiftKey ? 32 : 8;
        if (event.key === "ArrowRight") resize(width + step);
        else if (event.key === "ArrowLeft") resize(width - step);
        else if (event.key === "Home") resize(SIDEBAR_MIN_WIDTH);
        else if (event.key === "End") resize(SIDEBAR_MAX_WIDTH);
        else if (event.key === "Enter") resize(SIDEBAR_DEFAULT_WIDTH);
        else if (event.key === "Escape" && drag.current) {
          resize(drag.current.width);
          finish();
        } else return;
        event.preventDefault();
        event.stopPropagation();
      }}
    />
  );
}
