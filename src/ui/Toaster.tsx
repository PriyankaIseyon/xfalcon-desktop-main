import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { AlertTriangle, Check, Info, X } from "lucide-react";
import { cn } from "../lib/cn";
import { dismissToast, getToasts, subscribeToasts, type ToasterToast, type ToasterTone } from "./toastStore";
import { Button } from "./Button";
import { IconButton } from "./IconButton";

const MAX_VISIBLE = 3;
/** Matches the `--xf-dur-overlay` used by `app-toaster-out` in `index.css` —
 * the two must agree, or the row either snaps away before its fade finishes or
 * sits inert-but-visible after it. */
const EXIT_MS = 500;

/** Under reduced motion there is no fade to wait for, so a dismissed toast
 * leaves at once instead of sitting still and visible for the exit's length. */
function exitDelay(): number {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ? 0 : EXIT_MS;
}

/**
 * Keeps a just-dismissed toast mounted for one exit animation instead of
 * dropping it the instant `dismissToast` removes it from the store. The
 * store stays the single source of truth for which toasts are logically
 * live; this only delays the DOM removal of ones that just left it.
 *
 * Ordered by id (monotonic in `toastStore`), so a toast fading out settles
 * back into the position it already had instead of jumping to the end.
 */
function useDisplayedToasts(liveItems: ToasterToast[]): { item: ToasterToast; exiting: boolean }[] {
  const [exiting, setExiting] = useState<ToasterToast[]>([]);
  const prevItemsRef = useRef(liveItems);
  const timersRef = useRef(new Map<number, number>());

  // Diffed and applied DURING render (React's documented "adjusting state
  // when a prop changes" pattern), not in an effect. An effect runs after
  // this render has already committed and painted, so the removed toast
  // would be absent from the DOM for one real frame and then reappear to
  // start fading -- exactly the flash this feature exists to avoid.
  if (prevItemsRef.current !== liveItems) {
    const prevItems = prevItemsRef.current;
    prevItemsRef.current = liveItems;
    const liveIds = new Set(liveItems.map((item) => item.id));
    const newlyRemoved = prevItems.filter((item) => !liveIds.has(item.id));
    // A toast pushed out of the visible stack can come back when a newer one
    // is dismissed. Coming back ends its exit, so a later exit is a fresh one
    // -- one entry, one full fade -- rather than a second copy cut short by
    // the first exit's timer.
    const kept = exiting.filter((entry) => !liveIds.has(entry.id));
    const known = new Set(kept.map((entry) => entry.id));
    const added = newlyRemoved.filter((item) => !known.has(item.id));
    // Only on a real change: a render-phase update always renders again.
    if (kept.length !== exiting.length || added.length > 0) setExiting([...kept, ...added]);
  }

  // Scheduling the real removal stays in an effect: it is an outside-world
  // timer, not derived render output, and must not re-arm on every render.
  useEffect(() => {
    const exitingIds = new Set(exiting.map((item) => item.id));
    for (const [id, handle] of timersRef.current) {
      if (exitingIds.has(id)) continue;
      window.clearTimeout(handle);
      timersRef.current.delete(id);
    }
    for (const item of exiting) {
      if (timersRef.current.has(item.id)) continue;
      const handle = window.setTimeout(() => {
        timersRef.current.delete(item.id);
        setExiting((prev) => prev.filter((entry) => entry.id !== item.id));
      }, exitDelay());
      timersRef.current.set(item.id, handle);
    }
  }, [exiting]);

  useEffect(() => {
    const timers = timersRef.current;
    return () => {
      for (const handle of timers.values()) window.clearTimeout(handle);
    };
  }, []);

  const liveIds = new Set(liveItems.map((item) => item.id));
  return [
    ...liveItems.map((item) => ({ item, exiting: false })),
    ...exiting.filter((item) => !liveIds.has(item.id)).map((item) => ({ item, exiting: true })),
  ].sort((a, b) => a.item.id - b.item.id);
}

const TONE_ICON: Record<ToasterTone, typeof Info> = {
  message: Info,
  success: Check,
  warning: AlertTriangle,
  error: AlertTriangle,
};

function ToastCard({ item, paused, exiting }: { item: ToasterToast; paused: boolean; exiting: boolean }) {
  // Timer bookkeeping lives per-card so hovering the stack pauses every
  // remaining countdown and un-hovering resumes each from where it stopped.
  const remainingRef = useRef(item.duration);
  const startedAtRef = useRef<number | null>(null);
  const cardRef = useRef<HTMLDivElement | null>(null);

  // The exit collapses the card's own slot so the toasts above it glide down
  // into place instead of jumping once it is removed. Its height is measured
  // here, before the exit's first frame, because a keyframe cannot read it.
  useLayoutEffect(() => {
    if (exiting) cardRef.current?.style.setProperty("--xf-toast-exit-height", `${cardRef.current.offsetHeight}px`);
  }, [exiting]);

  useEffect(() => {
    // An exiting card is already leaving; it must not race its own auto-dismiss
    // timer into calling `dismissToast` again on an id the store no longer has.
    if (exiting || remainingRef.current === null || paused) return;
    startedAtRef.current = Date.now();
    const handle = setTimeout(() => dismissToast(item.id), remainingRef.current);
    return () => {
      clearTimeout(handle);
      if (startedAtRef.current !== null && remainingRef.current !== null) {
        remainingRef.current = Math.max(0, remainingRef.current - (Date.now() - startedAtRef.current));
      }
    };
  }, [item.id, paused, exiting]);

  const Icon = TONE_ICON[item.tone];
  return (
    <div
      aria-live={exiting ? undefined : item.tone === "error" ? "assertive" : "polite"}
      aria-hidden={exiting || undefined}
      className="app-toaster__toast"
      ref={cardRef}
      data-exiting={exiting || undefined}
      data-tone={item.tone}
      role={exiting ? undefined : item.tone === "error" ? "alert" : "status"}
    >
      <span aria-hidden className="app-toaster__icon">
        <Icon size={14} strokeWidth={2.2} />
      </span>
      <span className="app-toaster__text">{item.text}</span>
      {item.action ? (
        <Button
          className="app-toaster__action"
          size="sm"
          variant="ghost"
          onClick={() => {
            item.action?.onSelect();
            dismissToast(item.id);
          }}
        >
          {item.action.label}
        </Button>
      ) : null}
      <IconButton
        aria-label="Dismiss notification"
        className="app-toaster__close"
        size="sm"
        onClick={() => dismissToast(item.id)}
      >
        <X aria-hidden size={14} />
      </IconButton>
    </div>
  );
}

/**
 * The one notification stack for the app, mounted once at the app root.
 * Newest at the bottom, at most `MAX_VISIBLE` on screen (older ones wait
 * their turn), timers pause while the pointer or keyboard focus is in the stack.
 */
export function Toaster({ className }: { className?: string }) {
  const items = useSyncExternalStore(subscribeToasts, getToasts, getToasts);
  const [pointerPaused, setPointerPaused] = useState(false);
  const [focusPaused, setFocusPaused] = useState(false);
  const paused = pointerPaused || focusPaused;

  // Hooks run every render regardless of the early return below, so an
  // exiting toast keeps its own state (and finishes its fade) even on the
  // render where the store already reports zero live toasts.
  const displayed = useDisplayedToasts(items.slice(-MAX_VISIBLE));

  if (displayed.length === 0) return null;

  return (
    <div
      className={cn("app-toaster", className)}
      onFocusCapture={() => setFocusPaused(true)}
      onBlurCapture={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocusPaused(false);
      }}
      onPointerEnter={() => setPointerPaused(true)}
      onPointerLeave={() => setPointerPaused(false)}
    >
      {displayed.map(({ item, exiting }) => (
        <ToastCard exiting={exiting} item={item} key={item.id} paused={paused} />
      ))}
    </div>
  );
}
