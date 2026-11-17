import React from "react";

/**
 * One owner for Escape and Tab, chosen by depth rather than by registration
 * order.
 *
 * # The defect this exists to remove
 *
 * Modals and menus each installed their own `document` keydown listener in the
 * capture phase. Listeners on the same node in the same phase run in
 * REGISTRATION order, and a modal always registers before a menu opened inside
 * it — so the modal answered Escape first. Opening the format menu in the
 * export dialog and pressing Escape dismissed the whole dialog. The menu's
 * `stopPropagation` could not help: by the time it ran, the modal had already
 * closed.
 *
 * A stack fixes the ordering at the source. Layers push as they open and pop as
 * they close, and the TOPMOST layer that claims the key gets it — so the answer
 * to "what does Escape close" is always "the thing you opened last".
 *
 * # What counts as a layer
 *
 * Something that took the keyboard and has to give it back: a dialog, a menu.
 *
 * A tooltip is not one. It is transient, it is often open over a dialog by
 * accident of where the pointer rests, and the APG tooltip pattern dismisses it
 * on Escape WITHOUT claiming the keystroke. It keeps its own listener, and both
 * it and the top layer respond. `stopPropagation` below does not silence it,
 * because listeners on the same node in the same phase all run regardless.
 *
 * Escape handlers on `window` — canvas fullscreen, the app shortcut map — are a
 * third case. They sit below `document` in the propagation path, so they fall
 * silent exactly while a dialog or a menu is open, which is the arrangement
 * each of them separately tried to make in a comment.
 */
export interface DismissLayer {
  /** Omit to pass Escape to the layer below, as a dialog that refuses to be
   * dismissed does. */
  onEscape?: () => void;
  /** Omit to leave Tab to the browser. */
  onTab?: (event: KeyboardEvent) => void;
}

/**
 * Refs, not values. A layer holds its place in the stack for as long as it is
 * open, and reads its current handlers at dispatch time — re-pushing whenever a
 * callback identity changed would put a dialog back on top of the menu it
 * opened.
 */
const layers: React.RefObject<DismissLayer>[] = [];

function dispatch(event: KeyboardEvent) {
  if (event.key !== "Escape" && event.key !== "Tab") return;
  for (let index = layers.length - 1; index >= 0; index -= 1) {
    const layer = layers[index].current;
    const handler = event.key === "Escape" ? layer.onEscape : layer.onTab;
    if (handler == null) continue;
    event.stopPropagation();
    handler(event);
    return;
  }
}

/** The one listener, installed while at least one layer is open and removed
 * again when the last one closes. */
function push(layer: React.RefObject<DismissLayer>): () => void {
  if (layers.length === 0) document.addEventListener("keydown", dispatch, true);
  layers.push(layer);
  return () => {
    const index = layers.indexOf(layer);
    if (index !== -1) layers.splice(index, 1);
    if (layers.length === 0) document.removeEventListener("keydown", dispatch, true);
  };
}

/**
 * Claims Escape and Tab for as long as `active`, above every layer already open.
 *
 * Push order is open order because a layer is opened by an interaction with the
 * layer beneath it, which is a later commit. Two layers that first render in the
 * SAME commit would push child-first, and the outer one would win — no surface
 * opens a dialog and a menu together, and none should.
 */
export function useDismissLayer(active: boolean, layer: DismissLayer): void {
  const latest = React.useRef(layer);
  latest.current = layer;

  React.useEffect(() => {
    if (!active) return;
    return push(latest);
  }, [active]);
}
