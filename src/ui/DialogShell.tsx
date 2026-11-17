import React from "react";
import ReactDOM from "react-dom";
import { cn } from "../lib/cn";
import { useExitAnimation } from "./useExitAnimation";
import { useModalFocusTrap } from "./useModalFocusTrap";

export type DialogRole = "dialog" | "alertdialog";
export type DialogPlacement = "center" | "top";

export interface DialogShellProps {
  open: boolean;
  /**
   * `alertdialog` when the dialog reports a fault or asks for a decision that
   * cannot wait. A screen reader interrupts for one and queues the other.
   */
  role?: DialogRole;
  /** Where the panel sits. A command palette reads better high on the screen. */
  placement?: DialogPlacement;
  /**
   * Escape, and a scrim click unless `dismissOnScrim` says otherwise.
   * `undefined` disables both, which is how a dialog says it must be answered.
   */
  onDismiss?: () => void;
  /**
   * Whether a click on the scrim dismisses. A prompt that reports a broken
   * capability sets this false: a stray click must not be read as accepting
   * the fault.
   */
  dismissOnScrim?: boolean;
  labelledBy?: string;
  describedBy?: string;
  /** Only when there is no visible title to point `labelledBy` at. */
  label?: string;
  /** Classes for the panel. The shell owns the overlay and the scrim. */
  className?: string;
  panelRef?: React.Ref<HTMLDivElement>;
  onKeyDown?: React.KeyboardEventHandler<HTMLDivElement>;
  children: React.ReactNode;
}

/**
 * The one dialog shell: portal, scrim, placement, focus, and ARIA.
 *
 * Four surfaces used to carry their own copy of this -- `Modal`, `AlertDialog`,
 * the MCP reauthentication prompt, and the command palette -- and they had
 * drifted apart in ways that were defects rather than choices: two of them
 * dimmed the background with `bg-foreground/30`, which in the dark theme is
 * near-white ink and brightened the page instead; the palette hand-rolled its
 * own Tab trap and so never marked the app behind it `inert`, leaving it
 * reachable by a screen reader's rotor; and two hard-coded their title and
 * description element ids, which collide the moment two dialogs are open.
 *
 * What legitimately varies is the dismissal rule and the placement, so those
 * are props. Everything else is the same dialog.
 */
export function DialogShell({
  open,
  role = "dialog",
  placement = "center",
  onDismiss,
  dismissOnScrim = true,
  labelledBy,
  describedBy,
  label,
  className,
  panelRef,
  onKeyDown,
  children,
}: DialogShellProps) {
  const innerRef = React.useRef<HTMLDivElement | null>(null);
  // The whole overlay leaves together: the scrim fades while the panel settles.
  const exitRef = useExitAnimation<HTMLDivElement>();

  useModalFocusTrap(open, innerRef, onDismiss);

  const setPanel = React.useCallback(
    (node: HTMLDivElement | null) => {
      innerRef.current = node;
      if (typeof panelRef === "function") panelRef(node);
      else if (panelRef) (panelRef as React.RefObject<HTMLDivElement | null>).current = node;
    },
    [panelRef],
  );

  if (!open) return null;

  const scrimDismisses = dismissOnScrim && onDismiss !== undefined;

  return ReactDOM.createPortal(
    <div
      ref={exitRef}
      className={cn("dialog-shell", placement === "top" ? "dialog-shell--top" : "dialog-shell--center")}
    >
      <div
        aria-hidden
        className={cn("dialog-scrim xf-overlay-fade", scrimDismisses && "cursor-default")}
        onClick={scrimDismisses ? onDismiss : undefined}
      />
      <div
        ref={setPanel}
        role={role}
        aria-modal="true"
        aria-labelledby={labelledBy}
        aria-describedby={describedBy}
        aria-label={labelledBy == null ? label : undefined}
        tabIndex={-1}
        onKeyDown={onKeyDown}
        className={cn("relative focus:outline-none", className)}
      >
        {children}
      </div>
    </div>,
    document.body,
  );
}
