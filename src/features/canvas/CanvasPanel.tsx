import { useEffect, useRef, type KeyboardEvent } from "react";
import { BarChart3, Code2, Maximize2, Minimize2, PanelRightClose, Table2, X } from "lucide-react";
import { IconButton } from "../../ui/IconButton";
import {
  closeCanvas,
  closeCanvasArtifact,
  getCanvasState,
  setActiveCanvasTab,
  toggleCanvasExpanded,
  useCanvasStore,
  type CanvasArtifact,
} from "./canvasStore";
import { DataGrid } from "./DataGrid";
import { ChartCanvas } from "./ChartCanvas";
import { HtmlCanvas } from "./HtmlCanvas";

function ArtifactIcon({ artifact }: { artifact: CanvasArtifact }) {
  switch (artifact.type) {
    case "table":
      return <Table2 aria-hidden className="shrink-0 text-accent" size={13} />;
    case "chart":
      return <BarChart3 aria-hidden className="shrink-0 text-success" size={13} />;
    case "html":
      return <Code2 aria-hidden className="shrink-0 text-warn" size={13} />;
  }
}

/**
 * The Canvas: a tab strip over the open artifacts, and the active one below it
 * (spec: dynamic-canvas-artifacts).
 *
 * A real `tablist`, so the strip behaves the way a keyboard user expects rather
 * than merely being reachable: Left/Right and Home/End move the selection,
 * Delete closes a tab, and only the selected tab is in the tab order — a strip
 * of eight focus stops between the transcript and the artifact is a thicket you
 * have to Tab your way out of.
 */
export function CanvasPanel() {
  const { tabs, activeArtifactId, isOpen, isExpanded } = useCanvasStore();
  const stripRef = useRef<HTMLDivElement>(null);

  // Escape restores the previous split without closing the artifact.
  useEffect(() => {
    if (!isExpanded) return undefined;
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      toggleCanvasExpanded();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [isExpanded]);

  if (!isOpen || tabs.length === 0) return null;

  // `?? tabs[0]`: the store keeps these in step, and falling back beats
  // rendering an empty panel if they ever disagree.
  const active = tabs.find((tab) => tab.id === activeArtifactId) ?? tabs[0]!;
  const activeIndex = tabs.findIndex((tab) => tab.id === active.id);

  /** Moves selection and takes focus with it, which is what makes arrow keys
   * feel like a tab strip instead of a list of buttons. */
  const focusTab = (index: number) => {
    const bounded = (index + tabs.length) % tabs.length;
    const target = tabs[bounded];
    if (target === undefined) return;
    setActiveCanvasTab(target.id);
    stripRef.current?.querySelector<HTMLElement>(`[data-tab-id="${cssEscape(target.id)}"]`)?.focus();
  };

  const onStripKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    switch (event.key) {
      case "ArrowRight":
        event.preventDefault();
        focusTab(activeIndex + 1);
        break;
      case "ArrowLeft":
        event.preventDefault();
        focusTab(activeIndex - 1);
        break;
      case "Home":
        event.preventDefault();
        focusTab(0);
        break;
      case "End":
        event.preventDefault();
        focusTab(tabs.length - 1);
        break;
      case "Delete":
        event.preventDefault();
        closeTab(active.id);
        break;
      default:
        break;
    }
  };

  const closeTab = (id: string) => {
    closeCanvasArtifact(id);
    queueMicrotask(() => {
      const nextId = getCanvasState().activeArtifactId;
      if (nextId === null) return;
      stripRef.current?.querySelector<HTMLElement>(`[data-tab-id="${cssEscape(nextId)}"]`)?.focus();
    });
  };

  return (
    <div className="canvas-panel flex h-full w-full flex-col overflow-hidden border-l border-hairline bg-card">
      <div className="canvas-panel__header flex shrink-0 items-center justify-between border-b border-hairline bg-surface/50 px-2 backdrop-blur-xs">
        <div
          ref={stripRef}
          aria-label="Canvas artifacts"
          className="scrollbar-none flex min-w-0 flex-1 items-center gap-1 overflow-x-auto py-1"
          role="tablist"
          onKeyDown={onStripKeyDown}
        >
          {tabs.map((tab, index) => {
            const selected = tab.id === active.id;
            return (
              <div
                className={`canvas-panel__tab group flex h-8 max-w-[200px] shrink-0 items-center gap-1.5 px-2.5 transition-colors ${
                  selected
                    ? "border border-hairline bg-card text-foreground shadow-e1"
                    : "text-muted hover:bg-hover hover:text-foreground"
                }`}
                key={tab.id}
              >
                <button
                  aria-controls="canvas-artifact-panel"
                  aria-selected={selected}
                  className="flex min-w-0 flex-1 items-center gap-1.5 truncate text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[color:var(--xf-focus)]"
                  data-tab-id={tab.id}
                  id={`canvas-artifact-tab-${index}`}
                  role="tab"
                  // Only the selected tab is a focus stop; the arrow keys reach
                  // the rest.
                  tabIndex={selected ? 0 : -1}
                  type="button"
                  onClick={() => setActiveCanvasTab(tab.id)}
                >
                  <ArtifactIcon artifact={tab} />
                  <span className="truncate">{tab.title}</span>
                </button>
                {/* ui-extension: a 16 px control inside a tab. `IconButton`
                    starts at the 32 px compact size, which would push the tab
                    strip taller than the title it labels. */}
                <button
                  aria-label={`Close ${tab.title}`}
                  className="-mr-1 flex size-7 items-center justify-center rounded-sm text-faint opacity-60 transition-opacity hover:bg-hover hover:text-foreground group-hover:opacity-100"
                  // Outside the tab order for the same reason: Delete on the
                  // focused tab closes it, so this is the mouse's affordance.
                  tabIndex={-1}
                  type="button"
                  onClick={(event) => {
                    event.stopPropagation();
                    closeTab(tab.id);
                  }}
                >
                  <X aria-hidden size={11} />
                </button>
              </div>
            );
          })}
        </div>

        {/* The panel's own two controls: how big it is, and whether it is here.
            A viewport picker used to sit above the document instead; panel width
            is the honest size control, and this is how it gets more of it. */}
        <div className="flex shrink-0 items-center gap-1 border-l border-hairline/60 pl-2">
          <IconButton
            aria-label={isExpanded ? "Restore panel width" : "Expand panel"}
            aria-pressed={isExpanded}
            size="md"
            tooltip={isExpanded ? "Restore panel width (Esc)" : "Expand panel to 70%"}
            onClick={toggleCanvasExpanded}
          >
            {isExpanded ? <Minimize2 aria-hidden size={15} /> : <Maximize2 aria-hidden size={15} />}
          </IconButton>
          <IconButton aria-label="Close side panel" size="md" onClick={closeCanvas}>
            <PanelRightClose aria-hidden size={15} />
          </IconButton>
        </div>
      </div>

      {/* Keyed by artifact id, so switching tabs mounts a fresh renderer rather
          than handing the next artifact the previous one's state. Without it a
          grid keeps the filter and sort column from the table before it — and a
          sort column past the new table's width is a comparison against
          nothing. */}
      <div
        aria-label={active.title}
        aria-labelledby={`canvas-artifact-tab-${activeIndex}`}
        className="flex min-h-0 flex-1 flex-col overflow-hidden"
        id="canvas-artifact-panel"
        role="tabpanel"
      >
        {active.type === "table" ? <DataGrid artifact={active} key={active.id} /> : null}
        {active.type === "chart" ? <ChartCanvas artifact={active} key={active.id} /> : null}
        {active.type === "html" ? <HtmlCanvas artifact={active} key={active.id} /> : null}
      </div>
    </div>
  );
}

/**
 * Escapes an artifact id for a CSS attribute selector.
 *
 * Ids are composed from a model's own table titles and headers, so they contain
 * quotes and backslashes as a matter of course. `CSS.escape` is the right tool
 * and exists in every engine this app runs on; the manual fallback keeps the
 * lookup working under a test environment that has not defined it.
 */
function cssEscape(value: string): string {
  return typeof CSS !== "undefined" && typeof CSS.escape === "function"
    ? CSS.escape(value)
    : value.replace(/["\\]/g, "\\$&");
}
