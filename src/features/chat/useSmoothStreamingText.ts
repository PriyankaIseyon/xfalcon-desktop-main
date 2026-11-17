import { useLayoutEffect, useRef, useState } from "react";
import { advancePacer, revealEnd, startPacer } from "./revealPacer";

const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

function revealImmediately(): boolean {
  if (typeof window === "undefined" || typeof document === "undefined") return true;
  if (document.visibilityState === "hidden") return true;
  return typeof window.matchMedia === "function" && window.matchMedia(REDUCED_MOTION_QUERY).matches;
}

/**
 * Turns provider-sized delta bursts into a steady typewriter reveal.
 *
 * The source remains authoritative. Each frame reveals the amount `revealPacer`
 * allows: a fixed short lag behind arrival, a speed that eases rather than
 * jumps, and a backlog that still scales the speed so a large burst catches up.
 * When the provider finishes, the reveal drains what is left quickly instead of
 * dumping it in one frame. A different or replaced stream, a hidden document and
 * reduced motion show the authoritative source at once.
 */
export function useSmoothStreamingText(target: string, streaming: boolean, streamKey: string | null = null): string {
  const pacer = useRef(startPacer());
  const [display, setDisplay] = useState(() => ({
    key: streamKey,
    text: streaming && !revealImmediately() ? "" : target,
  }));
  const sameStream = display.key === streamKey;
  // The reveal runs while the provider streams, and after it ends for as long as
  // part of this same stream's answer is still unrevealed. A source that was never
  // streamed, or that replaced the revealed text, is shown whole.
  const unrevealed = sameStream && target.length > display.text.length && target.startsWith(display.text);
  const animating = (streaming || unrevealed) && !revealImmediately();
  const visible = !animating ? target : sameStream ? display.text : "";

  useLayoutEffect(() => {
    if (!sameStream) {
      pacer.current = startPacer();
      setDisplay({ key: streamKey, text: visible });
      return;
    }
    // `visible` already returns the authoritative source synchronously when the
    // reveal is over (and for reduced motion). Mirroring that value into state
    // here caused a second, visually identical React commit just before the
    // durable snapshot commit — the two or three full-response flashes seen at
    // completion. The next stream resets this cache from its new key, so no
    // terminal state write is needed.
    if (!animating) {
      pacer.current = startPacer();
      return;
    }
    // A retry or replacement is not an append. Clear the stale prefix before
    // revealing the new stream; never splice two answers together.
    if (!target.startsWith(visible)) {
      pacer.current = startPacer();
      setDisplay({ key: streamKey, text: "" });
      return;
    }
    // Caught up and waiting for more text. The next burst starts from rest.
    if (visible.length >= target.length) {
      pacer.current = startPacer(target.length);
      return;
    }

    let frame = 0;
    const reveal = (now: number) => {
      // Never let the pacer trail what is already on screen.
      const resumed =
        pacer.current.shown < visible.length ? { ...pacer.current, shown: visible.length } : pacer.current;
      pacer.current = advancePacer(resumed, now, target.length, !streaming);
      const end = revealEnd(target, pacer.current.shown);
      // A slow reveal advances less than one character on some frames. Keep
      // ticking; only a visible change needs a new render.
      if (end > visible.length) setDisplay({ key: streamKey, text: target.slice(0, end) });
      else frame = window.requestAnimationFrame(reveal);
    };
    frame = window.requestAnimationFrame(reveal);
    return () => window.cancelAnimationFrame(frame);
  }, [animating, sameStream, streamKey, streaming, target, visible]);

  return visible;
}
