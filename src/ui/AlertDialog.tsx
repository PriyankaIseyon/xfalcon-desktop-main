import React from "react";
import { Alert } from "./Alert";
import { Button } from "./Button";
import { DialogShell } from "./DialogShell";
import { SectionTitle } from "./Heading";

export interface AlertDialogProps {
  open: boolean;
  title: string;
  description: React.ReactNode;
  confirmLabel: string;
  cancelLabel?: string;
  destructive?: boolean;
  busy?: boolean;
  error?: string | null;
  /** Disables confirm without making the dialog busy, for a form that is not yet valid. */
  confirmDisabled?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  dismissOnScrim?: boolean;
  onKeyDown?: React.KeyboardEventHandler<HTMLDivElement>;
  dialogRef?: React.Ref<HTMLDivElement>;
  /** Body content after the description, for a dialog that needs more than a sentence. */
  children?: React.ReactNode;
}

/**
 * A decision the reader has to make before anything else happens.
 *
 * `alertdialog` rather than `dialog`, because a screen reader interrupts for
 * one and queues the other. Cancel takes focus: the non-destructive outcome is
 * the safe default for a stray Enter, and it is also what Escape and the scrim
 * do.
 */
export function AlertDialog({
  open,
  title,
  description,
  confirmLabel,
  cancelLabel = "Cancel",
  destructive = false,
  busy = false,
  error = null,
  confirmDisabled = false,
  onConfirm,
  onCancel,
  dismissOnScrim = true,
  onKeyDown,
  dialogRef,
  children,
}: AlertDialogProps) {
  // Not a constant: two alert dialogs can be open at once -- a sign-out
  // confirmation raised over a delete -- and a shared id points both at the
  // first one's heading.
  const id = React.useId();

  return (
    <DialogShell
      className="surface-card xf-material xf-overlay-pop alert-panel flex w-full max-w-md flex-col gap-4 p-5 shadow-e3"
      describedBy={`alert-dialog-description-${id}`}
      dismissOnScrim={dismissOnScrim}
      labelledBy={`alert-dialog-title-${id}`}
      onDismiss={busy ? undefined : onCancel}
      onKeyDown={onKeyDown}
      open={open}
      panelRef={dialogRef}
      role="alertdialog"
    >
      <SectionTitle className="shrink-0 break-words" id={`alert-dialog-title-${id}`} level={2}>
        {title}
      </SectionTitle>
      <div className="min-h-0 flex-auto overflow-y-auto overscroll-contain break-words space-y-4">
        <p className="type-body text-secondary" id={`alert-dialog-description-${id}`}>
          {description}
        </p>
        {children}
        {error ? (
          <Alert title="The action did not finish" tone="danger">
            {error}
          </Alert>
        ) : null}
      </div>
      <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 pt-2">
        <Button data-dialog-initial-focus="" disabled={busy} type="button" variant="secondary" onClick={onCancel}>
          {cancelLabel}
        </Button>
        <Button
          disabled={confirmDisabled}
          loading={busy}
          type="button"
          variant={destructive ? "destructive" : "primary"}
          onClick={onConfirm}
        >
          {confirmLabel}
        </Button>
      </div>
    </DialogShell>
  );
}
