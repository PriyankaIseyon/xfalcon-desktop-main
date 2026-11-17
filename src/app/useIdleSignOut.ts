import { useEffect, useRef } from "react";

/** AUTH-04 requires an automatic session timeout after inactivity; the product owner chose half a day. */
export const IDLE_SIGN_OUT_MS = 12 * 60 * 60 * 1000;
export const IDLE_CHECK_INTERVAL_MS = 60_000;
/** After a sign-out that did not happen, the next attempt waits this long. */
export const IDLE_RETRY_MS = 5 * 60 * 1000;
/**
 * An open dialog postpones the sign-out so typed input is not thrown away, but not
 * for ever: this long after the last real input the person is signed out even
 * with a dialog open. Running work is never cut short.
 */
export const IDLE_DIALOG_HOLD_MS = 24 * 60 * 60 * 1000;

/** Pointer and key bursts would otherwise write the clock on every frame. */
const ACTIVITY_THROTTLE_MS = 1_000;
const ACTIVITY_EVENTS = ["pointerdown", "pointermove", "keydown", "wheel", "touchstart"] as const;

/** Shown on the sign-in screen after an idle sign-out. */
export function idleSignOutNotice(): string {
  return "Signed out after 12 hours of inactivity. Sign in to continue.";
}

/**
 * Signs the person out after `IDLE_SIGN_OUT_MS` without input.
 *
 * Elapsed time always comes from `now()`, never from counted interval ticks:
 * a sleeping laptop freezes timers, and the first check after waking must see
 * the real gap.
 *
 * `onIdle` may return a promise. Resolving to `false`, or rejecting, means the
 * sign-out did not happen: the hook tries again after `IDLE_RETRY_MS`. Any other
 * result counts as done, and the hook stays fired while the shell unmounts.
 */
export function useIdleSignOut(options: {
  /** Called once when the idle limit is reached. */
  onIdle: () => void | Promise<boolean | void>;
  /** True while work the person started is still running; never sign out then. */
  busy: boolean;
  /** Checked on every tick, for state that lives outside React, such as an open dialog. */
  isBusy?: () => boolean;
  /** Test seam; defaults to Date.now. */
  now?: () => number;
}): void {
  const { onIdle, busy, isBusy } = options;
  const readNow = options.now ?? Date.now;

  // Timers and listeners read the newest callbacks through this ref, so a new
  // `onIdle` identity or a `busy` change never restarts the idle period.
  const latest = useRef({ onIdle, busy, isBusy, now: readNow });
  const lastActivity = useRef(0);
  // The last REAL input. `lastActivity` is also moved by postponements; this is not.
  const lastInput = useRef(0);
  const wasBusy = useRef(busy);
  const fired = useRef(false);

  useEffect(() => {
    // Work that ends starts the full period from the moment it ended. Otherwise
    // the period would count from the last check that still saw it running.
    if (wasBusy.current && !busy) {
      lastActivity.current = readNow();
    }
    wasBusy.current = busy;
    latest.current = { onIdle, busy, isBusy, now: readNow };
  });

  useEffect(() => {
    fired.current = false;
    lastActivity.current = latest.current.now();
    lastInput.current = lastActivity.current;

    const rearm = (): void => {
      const { now } = latest.current;
      fired.current = false;
      lastActivity.current = now() - IDLE_SIGN_OUT_MS + IDLE_RETRY_MS;
      lastInput.current = now() - IDLE_DIALOG_HOLD_MS + IDLE_RETRY_MS;
    };

    const check = (): void => {
      if (fired.current) return;
      const { onIdle: notify, busy: working, isBusy: dialogOpen, now } = latest.current;
      const current = now();
      // A clock that moved backwards has no valid baseline, so restart from here.
      if (current < lastActivity.current || current < lastInput.current) {
        lastActivity.current = current;
        lastInput.current = current;
        return;
      }
      // Running work always postpones. An open dialog postpones until a full day
      // without input, so a dialog left open cannot disable the timeout for good.
      const dialogHolds = dialogOpen?.() === true && current - lastInput.current < IDLE_DIALOG_HOLD_MS;
      if (working || dialogHolds) {
        lastActivity.current = current;
        return;
      }
      if (current - lastActivity.current >= IDLE_SIGN_OUT_MS || dialogOpen?.() === true) {
        fired.current = true;
        const outcome = notify();
        if (outcome instanceof Promise) {
          outcome.then((signedOut) => {
            if (signedOut === false) rearm();
          }, rearm);
        }
      }
    };

    const onActivity = (): void => {
      const current = latest.current.now();
      // Measured from the last REAL input: a postponement also moves `lastActivity`,
      // and input right after one must still count.
      const elapsed = current - lastInput.current;
      // Only forward movement is throttled; a backwards clock must still be recorded.
      if (elapsed >= 0 && elapsed < ACTIVITY_THROTTLE_MS) return;
      lastActivity.current = current;
      lastInput.current = current;
    };

    // Visibility and focus are checks, not activity: returning to the app must
    // not reset the clock on its own.
    const onVisibility = (): void => {
      if (document.visibilityState === "visible") check();
    };

    const interval = window.setInterval(check, IDLE_CHECK_INTERVAL_MS);
    for (const type of ACTIVITY_EVENTS) {
      window.addEventListener(type, onActivity, { passive: true });
    }
    window.addEventListener("focus", check);
    window.addEventListener("online", check);
    document.addEventListener("visibilitychange", onVisibility);

    return function teardown(): void {
      window.clearInterval(interval);
      for (const type of ACTIVITY_EVENTS) {
        window.removeEventListener(type, onActivity);
      }
      window.removeEventListener("focus", check);
      window.removeEventListener("online", check);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, []);
}
