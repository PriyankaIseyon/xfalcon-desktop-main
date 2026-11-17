import React from "react";

/**
 * Plays a floating surface's exit after React has already removed it.
 *
 * Every menu, tooltip, palette, and dialog here unmounts the moment it closes:
 * `ContextMenu` renders nothing for a null anchor, `DialogShell` nothing for
 * `open={false}`, and a dozen callers drop their dialog outright
 * (`{exportOpen ? <ChatExportDialog /> : null}`). Holding each of those mounted
 * for an exit phase would thread that phase through every caller, and a caller
 * that clears the data its dialog reads (`pendingChatDelete`) would repaint the
 * leaving dialog with empty text.
 *
 * So what leaves is a copy. The ref's cleanup runs while the node is still in
 * the document -- React detaches a ref before it removes the node -- and clones
 * it exactly as the reader last saw it: mid-entrance opacity and scale, scroll
 * offsets, field values. The copy goes back where the node was, marked
 * `data-xf-leaving`, which `index.css` animates out; it removes itself when its
 * animations finish. It is inert, hidden from assistive tech, carries no ids,
 * and takes no pointer events, so the app behind it is live from the first
 * frame of the exit.
 *
 * Nothing is left behind when there is nothing to play: under reduced motion
 * the global kill switch sets `animation: none`, and without Web Animations
 * (jsdom) there is no way to know when an exit ends.
 *
 * Returns a stable ref callback that also keeps `nodeRef` current, so a
 * surface that already holds its node in a ref needs no second one.
 */
export function useExitAnimation<T extends HTMLElement>(nodeRef?: React.RefObject<T | null>): React.RefCallback<T> {
  return React.useCallback(
    (node: T | null) => {
      if (nodeRef) nodeRef.current = node;
      if (node == null) return;
      return () => {
        if (nodeRef) nodeRef.current = null;
        leaveBehind(node);
      };
    },
    [nodeRef],
  );
}

function leaveBehind(node: HTMLElement) {
  const parent = node.parentNode;
  if (parent == null || typeof node.getAnimations !== "function") return;
  const next = node.nextSibling;
  const ghost = node.cloneNode(true) as HTMLElement;
  const scrolled = copyLiveState(node, ghost);
  ghost.setAttribute("data-xf-leaving", "");
  ghost.setAttribute("aria-hidden", "true");
  ghost.inert = true;

  // A microtask, so the question "did React actually remove it?" is asked after
  // the commit. A node still in the document was re-attached -- StrictMode runs
  // every ref cleanup once on mount -- and nothing is leaving.
  queueMicrotask(() => {
    if (node.isConnected || !parent.isConnected) return;
    parent.insertBefore(ghost, next?.parentNode === parent ? next : null);
    for (const [element, top, left] of scrolled) {
      element.scrollTop = top;
      element.scrollLeft = left;
    }
    // An infinite animation inside the copy -- a spinner in a busy button --
    // never finishes, so only the finite ones decide when the exit is over.
    const exits = ghost
      .getAnimations({ subtree: true })
      .filter((animation) => Number.isFinite(Number(animation.effect?.getComputedTiming().endTime)));
    if (exits.length === 0) {
      ghost.remove();
      return;
    }
    void Promise.allSettled(exits.map((animation) => animation.finished)).then(() => ghost.remove());
  });
}

/**
 * Carries over what `cloneNode` drops: what the reader was looking at, not what
 * the markup says. Returns the scroll offsets to restore, which only take once
 * the copy is laid out in the document.
 */
function copyLiveState(node: HTMLElement, ghost: HTMLElement): [Element, number, number][] {
  const originals: Element[] = [node, ...node.querySelectorAll("*")];
  const copies: Element[] = [ghost, ...ghost.querySelectorAll("*")];
  const scrolled: [Element, number, number][] = [];

  // A close in the middle of an entrance leaves from where the entrance got to.
  // The exit keyframes name only their end, so they start from these values
  // rather than snapping back to fully shown first.
  for (const animation of node.getAnimations({ subtree: true })) {
    const target = (animation.effect as KeyframeEffect | null)?.target ?? null;
    const index = target == null ? -1 : originals.indexOf(target);
    const copy = copies[index];
    if (target == null || !(copy instanceof HTMLElement || copy instanceof SVGElement)) continue;
    const { opacity, transform } = getComputedStyle(target);
    copy.style.opacity = opacity;
    copy.style.transform = transform;
  }

  originals.forEach((original, index) => {
    const copy = copies[index];
    // Two elements with one id would answer `getElementById` and every
    // `aria-labelledby` with the copy, and the next open reuses the same ids.
    // An SVG keeps its ids: a gradient or clip path inside the copy is found by
    // one, and the original that also held it is gone.
    if (!(original instanceof SVGElement)) copy.removeAttribute("id");
    // A leaving dialog is not a modal any more. Anything that asks "is a modal
    // open?" -- the composer, before it takes focus back -- must not find the
    // copy and answer yes for the length of its fade.
    copy.removeAttribute("aria-modal");
    if (original.scrollTop !== 0 || original.scrollLeft !== 0) {
      scrolled.push([copy, original.scrollTop, original.scrollLeft]);
    }
    if (
      original instanceof HTMLInputElement ||
      original instanceof HTMLTextAreaElement ||
      original instanceof HTMLSelectElement
    ) {
      const field = copy as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement;
      field.value = original.value;
      if (original instanceof HTMLInputElement) (field as HTMLInputElement).checked = original.checked;
      // A copied radio keeps its group name, and checking it would uncheck the
      // live radio that shares it.
      field.removeAttribute("name");
    }
    // A copied frame would load its document again for a surface that is
    // leaving.
    if (original instanceof HTMLIFrameElement) {
      copy.removeAttribute("src");
      copy.removeAttribute("srcdoc");
    }
  });
  return scrolled;
}
