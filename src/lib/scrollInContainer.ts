/**
 * Move `child` inside `container` without touching any other scroller.
 *
 * `Element.scrollIntoView` walks every overflow ancestor. In this shell that
 * includes the sidebar lists, so a jump to Settings Support also hid Chat and
 * Projects. Setting `container.scrollTop` is the one scroller we mean.
 */
export function scrollChildIntoContainer(container: HTMLElement, child: HTMLElement, paddingPx = 20): void {
  const containerRect = container.getBoundingClientRect();
  const childRect = child.getBoundingClientRect();
  const alreadyVisible =
    childRect.top >= containerRect.top + paddingPx && childRect.bottom <= containerRect.bottom - paddingPx;
  if (alreadyVisible) return;

  const nextTop = container.scrollTop + (childRect.top - containerRect.top) - paddingPx;
  container.scrollTop = Math.max(0, nextTop);
}
