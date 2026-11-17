import React from "react";

type ElasticSelectionOptions = {
  containerRef: React.RefObject<HTMLElement | null>;
  itemRefs: React.MutableRefObject<Array<HTMLButtonElement | null>>;
  selectedIndex: number;
  enabledIndexes: number[];
  itemKey: string;
  onSelect: (index: number) => void;
};

type Box = { left: number; top: number; width: number; height: number };
type DragState = { pointerId: number; startX: number; grabOffset: number; moved: boolean };
type DragClick = { clientX: number; clientY: number; timeStamp: number };

const REDUCED_MOTION = "(prefers-reduced-motion: reduce)";

/** Shares the measured, decorative selection thumb used by tab and radio strips. */
export function useElasticSelection({
  containerRef,
  itemRefs,
  selectedIndex,
  enabledIndexes,
  itemKey,
  onSelect,
}: ElasticSelectionOptions) {
  const thumbRef = React.useRef<HTMLSpanElement>(null);
  const boxesRef = React.useRef<Box[]>([]);
  const selectedIndexRef = React.useRef(selectedIndex);
  const enabledIndexesRef = React.useRef(enabledIndexes);
  const onSelectRef = React.useRef(onSelect);
  const animationRef = React.useRef<Animation | null>(null);
  const dragRef = React.useRef<DragState | null>(null);
  const dragClickRef = React.useRef<DragClick | null>(null);
  const reconciliationFrameRef = React.useRef<number | null>(null);
  const hasMeasuredRef = React.useRef(false);
  const previousSelectedIndexRef = React.useRef(selectedIndex);
  const reducedMotionRef = React.useRef(false);

  selectedIndexRef.current = selectedIndex;
  enabledIndexesRef.current = enabledIndexes;
  onSelectRef.current = onSelect;

  const cancelReconciliation = React.useCallback(() => {
    if (reconciliationFrameRef.current !== null) {
      cancelAnimationFrame(reconciliationFrameRef.current);
      reconciliationFrameRef.current = null;
    }
  }, []);

  const measure = React.useCallback(() => {
    const container = containerRef.current;
    if (!container) return;
    boxesRef.current = itemRefs.current.map((item) => {
      if (!item) return { left: 0, top: 0, width: 0, height: 0 };
      // Layout coordinates exclude the button's transient pressed transform.
      return {
        left: item.offsetLeft,
        top: item.offsetTop,
        width: item.offsetWidth,
        height: item.offsetHeight,
      };
    });
  }, [containerRef, itemRefs]);

  const place = React.useCallback(
    (index: number, animateMove: boolean) => {
      const thumb = thumbRef.current;
      const box = boxesRef.current[index];
      if (!thumb || !box || box.width <= 0 || box.height <= 0) {
        animationRef.current?.cancel();
        animationRef.current = null;
        if (thumb) {
          thumb.style.opacity = "0";
          thumb.style.transform = "none";
        }
        return;
      }
      const container = containerRef.current;
      const containerRect = container?.getBoundingClientRect();
      const currentRect = animationRef.current && containerRect ? thumb.getBoundingClientRect() : null;
      const inlineLeft = Number.parseFloat(thumb.style.left);
      const inlineTop = Number.parseFloat(thumb.style.top);
      const inlineWidth = Number.parseFloat(thumb.style.width);
      const inlineHeight = Number.parseFloat(thumb.style.height);
      const from =
        currentRect && containerRect
          ? {
              left: currentRect.left - containerRect.left - (container?.clientLeft ?? 0) + (container?.scrollLeft ?? 0),
              top: currentRect.top - containerRect.top - (container?.clientTop ?? 0) + (container?.scrollTop ?? 0),
              width: currentRect.width,
              height: currentRect.height,
            }
          : {
              left: Number.isFinite(inlineLeft) ? inlineLeft : box.left,
              top: Number.isFinite(inlineTop) ? inlineTop : box.top,
              width: Number.isFinite(inlineWidth) ? inlineWidth : box.width,
              height: Number.isFinite(inlineHeight) ? inlineHeight : box.height,
            };
      animationRef.current?.cancel();
      animationRef.current = null;
      thumb.style.left = `${box.left}px`;
      thumb.style.top = `${box.top}px`;
      thumb.style.width = `${box.width}px`;
      thumb.style.height = `${box.height}px`;
      thumb.style.opacity = "1";
      if (!animateMove || reducedMotionRef.current || typeof thumb.animate !== "function") return;

      const stretch = Math.min(14, Math.abs(box.left - from.left) * 0.14);
      const deltaX = from.left - box.left;
      const deltaY = from.top - box.top + (from.height - box.height) / 2;
      const minLeft = Math.min(from.left, box.left);
      const maxRight = Math.max(from.left + from.width, box.left + box.width);
      const middleWidth = Math.min(maxRight - minLeft, (from.width + box.width) / 2 + stretch);
      const middleLeft = Math.max(minLeft, Math.min(maxRight - middleWidth, (from.left + box.left) / 2 - stretch / 2));
      thumb.style.transformOrigin = "left center";
      animationRef.current = thumb.animate(
        [
          {
            transform: `translate(${deltaX}px, ${deltaY}px) scale(${from.width / box.width}, ${from.height / box.height})`,
            borderRadius: "var(--xf-r-sm)",
          },
          {
            // Stretch between the old and new slot, never beyond either edge:
            // transformed overflow would otherwise create a transient scrollbar.
            transform: `translate(${middleLeft - box.left}px, ${deltaY / 2}px) scale(${middleWidth / box.width}, ${(from.height + box.height) / (2 * box.height)})`,
            borderRadius: "var(--xf-r-md)",
            offset: 0.5,
          },
          { transform: "none", borderRadius: "var(--xf-r-sm)" },
        ],
        { duration: 360, easing: "cubic-bezier(.4,0,.2,1)" },
      );
    },
    [containerRef],
  );

  React.useLayoutEffect(() => {
    const container = containerRef.current;
    const thumb = thumbRef.current;
    if (!container || !thumb) return;
    measure();
    const query = window.matchMedia?.(REDUCED_MOTION);
    reducedMotionRef.current = query?.matches ?? false;
    const onMotionChange = (event: MediaQueryListEvent) => {
      reducedMotionRef.current = event.matches;
      if (event.matches) {
        cancelReconciliation();
        place(selectedIndexRef.current, false);
      }
    };
    query?.addEventListener?.("change", onMotionChange);
    const remeasure = () => {
      const previous = boxesRef.current;
      measure();
      if (
        boxesRef.current.some((box, index) => {
          const old = previous[index];
          return (
            !old || box.left !== old.left || box.top !== old.top || box.width !== old.width || box.height !== old.height
          );
        })
      )
        place(selectedIndexRef.current, false);
    };
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(remeasure);
    observer?.observe(container);
    itemRefs.current.forEach((item) => item && observer?.observe(item));
    document.fonts?.ready.then(() => {
      if (containerRef.current === container) {
        remeasure();
      }
    });
    if (selectedIndexRef.current >= 0) place(selectedIndexRef.current, false);
    hasMeasuredRef.current = true;
    return () => {
      observer?.disconnect();
      query?.removeEventListener?.("change", onMotionChange);
      cancelReconciliation();
      animationRef.current?.cancel();
      animationRef.current = null;
    };
    // Re-measure when the item set changes; ResizeObserver
    // handles label, font, and container size changes between those renders.
  }, [cancelReconciliation, containerRef, itemRefs, measure, place, itemKey]);

  React.useLayoutEffect(() => {
    const previous = previousSelectedIndexRef.current;
    previousSelectedIndexRef.current = selectedIndex;
    if (selectedIndex < 0) {
      dragRef.current = null;
      cancelReconciliation();
      place(-1, false);
      return;
    }
    if (selectedIndex === previous) return;
    dragRef.current = null;
    cancelReconciliation();
    measure();
    place(selectedIndex, hasMeasuredRef.current);
  }, [cancelReconciliation, measure, selectedIndex, place]);

  const onPointerDown = React.useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      if (event.button !== 0 || selectedIndexRef.current < 0 || dragRef.current) return;
      const selected = itemRefs.current[selectedIndexRef.current];
      if (!selected || !event.target || !selected.contains(event.target as Node)) return;
      cancelReconciliation();
      dragClickRef.current = null;
      const selectedRect = selected.getBoundingClientRect();
      dragRef.current = {
        pointerId: event.pointerId,
        startX: event.clientX,
        grabOffset: event.clientX - selectedRect.left,
        moved: false,
      };
      try {
        event.currentTarget.setPointerCapture(event.pointerId);
      } catch {
        // Some browsers do not permit capture for synthesized pointer events.
      }
    },
    [cancelReconciliation, itemRefs],
  );

  const onPointerMove = React.useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      if (!drag.moved && Math.abs(event.clientX - drag.startX) < 4) return;
      drag.moved = true;
      const container = containerRef.current;
      const thumb = thumbRef.current;
      const selectedBox = boxesRef.current[selectedIndexRef.current];
      if (!container || !thumb || !selectedBox) return;
      if (animationRef.current) {
        animationRef.current.cancel();
        animationRef.current = null;
      }
      const containerRect = container.getBoundingClientRect();
      const nextLeft =
        event.clientX - containerRect.left - container.clientLeft - drag.grabOffset + container.scrollLeft;
      const maxLeft = Math.max(0, container.scrollWidth - selectedBox.width);
      thumb.style.left = `${Math.min(maxLeft, Math.max(0, nextLeft))}px`;
    },
    [containerRef],
  );

  const onPointerEnd = React.useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      dragRef.current = null;
      if (!drag.moved) return;
      dragClickRef.current = { clientX: event.clientX, clientY: event.clientY, timeStamp: event.timeStamp };
      const enabled = enabledIndexesRef.current;
      const container = containerRef.current;
      if (enabled.length === 0 || !container) {
        place(selectedIndexRef.current, false);
        return;
      }
      const from = selectedIndexRef.current;
      const x = event.clientX;
      let nearest = enabled[0]!;
      let distance = Number.POSITIVE_INFINITY;
      for (const index of enabled) {
        const box = boxesRef.current[index];
        if (!box) continue;
        const containerLeft = container.getBoundingClientRect().left;
        const center = containerLeft + container.clientLeft + box.left - container.scrollLeft + box.width / 2;
        const nextDistance = Math.abs(center - x);
        if (nextDistance < distance) {
          nearest = index;
          distance = nextDistance;
        }
      }
      if (nearest !== from) onSelectRef.current(nearest);
      reconciliationFrameRef.current = requestAnimationFrame(() => {
        reconciliationFrameRef.current = null;
        const actual = selectedIndexRef.current;
        if (nearest !== from && actual === nearest) return;
        place(actual, true);
      });
    },
    [containerRef, place],
  );

  const onPointerCancel = React.useCallback(
    (event: React.PointerEvent<HTMLElement>) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      dragRef.current = null;
      cancelReconciliation();
      place(selectedIndexRef.current, true);
    },
    [cancelReconciliation, place],
  );

  const consumeClick = React.useCallback((event: React.MouseEvent<HTMLElement>) => {
    const release = dragClickRef.current;
    dragClickRef.current = null;
    const nativeEvent = event.nativeEvent;
    const suppressed = Boolean(
      release &&
      nativeEvent.isTrusted &&
      event.detail > 0 &&
      Math.abs(event.clientX - release.clientX) <= 2 &&
      Math.abs(event.clientY - release.clientY) <= 2 &&
      event.timeStamp - release.timeStamp <= 100,
    );
    return suppressed;
  }, []);

  return {
    thumbRef,
    onPointerDown,
    onPointerMove,
    onPointerUp: onPointerEnd,
    onPointerCancel,
    onLostPointerCapture: onPointerCancel,
    consumeClick,
  };
}
