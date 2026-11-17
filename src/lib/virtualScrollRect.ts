import type { Virtualizer } from "@tanstack/react-virtual";

/**
 * `observeElementRect` for `useVirtualizer` that never reports a zero-sized
 * viewport.
 *
 * A virtualizer whose scroll element measures 0x0 renders no items at all, which
 * happens in any environment without layout (jsdom, and the first paint of a
 * pane that has not been sized yet). Substituting a plausible viewport keeps the
 * list populated instead of silently empty; real measurements replace it as soon
 * as the `ResizeObserver` fires.
 */
export function observeScrollRect<T extends Element>(
  instance: Virtualizer<T, Element>,
  cb: (rect: { width: number; height: number }) => void,
) {
  const element = instance.scrollElement;
  if (!element) return;

  const measure = () => {
    const rect = element.getBoundingClientRect();
    const height = rect.height > 0 ? rect.height : 480;
    const width = rect.width > 0 ? rect.width : 640;
    cb({ width: Math.round(width), height: Math.round(height) });
  };

  measure();
  if (typeof ResizeObserver === "undefined") {
    return () => {};
  }
  const observer = new ResizeObserver(measure);
  observer.observe(element);
  return () => observer.disconnect();
}
