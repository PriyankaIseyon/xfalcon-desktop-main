import React from "react";
import { useDismissLayer } from "./dismissLayers";

// ---------------------------------------------------------------------------
// The focus behavior every modal-like overlay in this app shares. It lives in
// its own module rather than beside `Modal`: a file that exports both a
// component and a hook loses Fast Refresh for the component, so the two ship
// separately.
// ---------------------------------------------------------------------------

const FOCUSABLE =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Background inertness while any modal is open, ref-counted so a dialog
 * opened from within another (e.g. a sign-out confirmation) doesn't re-enable
 * the app root the moment the inner one closes.
 *
 * The Tab-trap below stops keyboard users at the panel edge, but a screen
 * reader's virtual cursor ignores Tab order entirely -- without `inert` on
 * `#root`, swiping/rotor navigation can still reach and activate content
 * behind a dialog that is supposedly modal. `inert` is already this
 * codebase's own pattern for "not here right now" (`Sidebar`'s collapsed
 * state), just applied to the app root instead of one panel.
 */
let openModalCount = 0;

function lockBackground() {
  openModalCount += 1;
  if (openModalCount !== 1) return;
  const root = document.getElementById("root");
  root?.setAttribute("inert", "");
  root?.setAttribute("aria-hidden", "true");
}

function unlockBackground() {
  openModalCount = Math.max(0, openModalCount - 1);
  if (openModalCount !== 0) return;
  const root = document.getElementById("root");
  root?.removeAttribute("inert");
  root?.removeAttribute("aria-hidden");
}

/**
 * The behavior every modal-like overlay in this app needs: background
 * `inert`, focus moved into the panel on open and restored to the opener on
 * close, and a Tab/Shift+Tab trap that keeps keyboard focus inside it.
 *
 * Shared so a dialog with different scrim behavior (`McpReauthPrompt`'s
 * refusal prompt does not dismiss on a stray scrim click, which this hook has
 * no opinion on) still gets the same trap and background-lock rather than a
 * second, hand-rolled copy that quietly drifts from this one.
 *
 * `onEscape` is a callback, not a boolean, so a caller can gate it on its own
 * state (`McpReauthPrompt` disables Escape while a reconnect is in flight) by
 * passing `undefined` instead of a handler.
 */
export function useModalFocusTrap(open: boolean, panelRef: React.RefObject<HTMLElement | null>, onEscape?: () => void) {
  const restoreFocusRef = React.useRef<HTMLElement | null>(null);

  // Capture the previously-focused element, lock the background, and move
  // focus into the panel.
  React.useEffect(() => {
    if (!open) return;
    restoreFocusRef.current = document.activeElement as HTMLElement | null;
    lockBackground();

    // Focus after the panel paints so the target nodes exist.
    const raf = window.requestAnimationFrame(() => {
      const panel = panelRef.current;
      if (!panel) return;
      // The user may already be typing in a field by the time this fires —
      // yanking focus to the first control mid-keystroke turns their next
      // space into a click on whatever it landed on (the close button).
      if (panel.contains(document.activeElement)) return;
      // A decision can prefer Cancel over a link in its description. Moving
      // focus here, after capturing the opener, also preserves return focus.
      const first =
        panel.querySelector<HTMLElement>("[data-dialog-initial-focus]:not([disabled])") ??
        panel.querySelector<HTMLElement>(FOCUSABLE);
      (first ?? panel).focus();
    });

    return () => {
      window.cancelAnimationFrame(raf);
      unlockBackground();
      // Restore focus to the opener on close/unmount — but only if it is still
      // in the document. A closed dialog often unmounts the trigger (e.g. a row
      // action that no longer exists), and focusing a detached node throws/no-ops
      // and can scroll the page; skip it in that case.
      const opener = restoreFocusRef.current;
      if (opener && opener.isConnected) opener.focus();
    };
  }, [open, panelRef]);

  // ESC to close + Tab/Shift+Tab trap within the panel. Both go through the
  // shared layer stack rather than a listener of this hook's own, so a menu
  // opened INSIDE the dialog answers first -- see `dismissLayers.ts`.
  useDismissLayer(open, {
    onEscape,
    onTab: (event: KeyboardEvent) => {
      const panel = panelRef.current;
      if (!panel) return;
      // A menu opened inside the dialog leaves an inert copy of itself in the
      // panel while it fades. `inert` does not hide it from a selector, and
      // an inert control cannot take focus, so a copy chosen as the first or
      // last stop would let Tab escape the trap or leave Shift+Tab stuck.
      const candidates = Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
        (element) => element.closest("[data-xf-leaving]") === null,
      );
      // `offsetParent` is the cheap test for "laid out", and it skips a control
      // inside a collapsed section. It is also null for everything in an
      // environment with no layout, which would leave the trap pinning focus on
      // whatever holds it -- a trap that quietly stops trapping. Falling back to
      // the unfiltered list keeps the real filter and refuses the degenerate
      // case.
      const visible = candidates.filter((el) => el.offsetParent !== null || el === document.activeElement);
      const focusable = visible.length > 1 ? visible : candidates;
      if (focusable.length === 0) {
        // Nothing tabbable — keep focus pinned on the panel.
        event.preventDefault();
        panel.focus();
        return;
      }
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      const active = document.activeElement;
      if (event.shiftKey) {
        if (active === first || !panel.contains(active)) {
          event.preventDefault();
          last.focus();
        }
      } else if (active === last || !panel.contains(active)) {
        event.preventDefault();
        first.focus();
      }
    },
  });
}
