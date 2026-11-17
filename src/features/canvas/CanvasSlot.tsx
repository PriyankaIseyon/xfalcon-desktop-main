import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { setCanvasExpanded } from "./canvasStore";

export interface CanvasSlotProps {
  ariaLabel?: string;
  children: ReactNode;
  open: boolean;
  expanded?: boolean;
}

const MIN_WIDTH = 20;
const MAX_WIDTH = 70;
const DEFAULT_WIDTH = 42;
const clamp = (value: number) => Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, value));

export function CanvasSlot({ ariaLabel = "Side panel", children, open, expanded = false }: CanvasSlotProps) {
  const panelRef = useRef<HTMLElement>(null);
  const drag = useRef<{ x: number; width: number; shellWidth: number; shell: HTMLElement } | null>(null);
  const [width, setWidth] = useState(DEFAULT_WIDTH);

  useLayoutEffect(() => {
    const shell = panelRef.current?.closest<HTMLElement>(".app-shell");
    shell?.style.setProperty("--xf-canvas-user-width", `${width}%`);
  }, [open, width]);

  useEffect(
    () => () => {
      // Closing the last tab or switching conversation during a drag must release
      // the iframe shield and selection lock even when pointerup cannot arrive.
      if (drag.current) delete drag.current.shell.dataset.canvasResizing;
      drag.current = null;
    },
    [open],
  );

  if (!open) return null;

  const finish = () => {
    if (drag.current) delete drag.current.shell.dataset.canvasResizing;
    drag.current = null;
  };
  const resize = (next: number) => {
    setCanvasExpanded(false);
    setWidth(clamp(next));
  };

  return (
    <aside
      ref={panelRef}
      id="artifact-side-panel"
      aria-label={ariaLabel}
      className="canvas-slot min-h-0 min-w-0 bg-card"
    >
      <div
        className="canvas-divider"
        role="separator"
        aria-label={ariaLabel}
        aria-controls="artifact-side-panel"
        aria-description="Drag or use Left and Right arrows to resize. Double-click or press Enter to reset."
        aria-orientation="vertical"
        aria-valuemin={MIN_WIDTH}
        aria-valuemax={MAX_WIDTH}
        aria-valuenow={Math.round(expanded ? MAX_WIDTH : width)}
        aria-valuetext={`${Math.round(expanded ? MAX_WIDTH : width)}% of window width`}
        tabIndex={0}
        onDoubleClick={() => resize(DEFAULT_WIDTH)}
        onPointerDown={(event) => {
          if (event.button !== 0 || !event.isPrimary) return;
          const shell = panelRef.current?.closest<HTMLElement>(".app-shell");
          if (!shell || !panelRef.current) return;
          const shellWidth = shell.getBoundingClientRect().width;
          if (shellWidth <= 0) return;
          event.preventDefault();
          event.currentTarget.focus();
          event.currentTarget.setPointerCapture(event.pointerId);
          drag.current = {
            x: event.clientX,
            width: (panelRef.current.getBoundingClientRect().width / shellWidth) * 100,
            shellWidth,
            shell,
          };
          shell.dataset.canvasResizing = "true";
        }}
        onPointerMove={(event) => {
          const start = drag.current;
          if (!start) return;
          resize(start.width + ((start.x - event.clientX) / start.shellWidth) * 100);
        }}
        onPointerUp={(event) => {
          finish();
          if (event.currentTarget.hasPointerCapture(event.pointerId))
            event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={finish}
        onLostPointerCapture={finish}
        onKeyDown={(event) => {
          const current = expanded ? MAX_WIDTH : width;
          const step = event.shiftKey ? 5 : 1;
          if (event.key === "ArrowLeft") resize(current + step);
          else if (event.key === "ArrowRight") resize(current - step);
          else if (event.key === "Home") resize(MIN_WIDTH);
          else if (event.key === "End") resize(MAX_WIDTH);
          else if (event.key === "Enter") resize(DEFAULT_WIDTH);
          else if (event.key === "Escape" && drag.current) {
            resize(drag.current.width);
            finish();
          } else return;
          event.preventDefault();
          event.stopPropagation();
        }}
      >
        <span className="canvas-divider__grip" aria-hidden="true">
          <span />
          <span />
          <span />
        </span>
      </div>
      <div className="h-full min-h-0 min-w-0">{children}</div>
    </aside>
  );
}
