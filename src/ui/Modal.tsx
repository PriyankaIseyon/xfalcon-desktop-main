import React from "react";
import { X } from "lucide-react";
import { cn } from "../lib/cn";
import { SectionTitle } from "./Heading";
import { DialogShell, type DialogRole } from "./DialogShell";
import { IconButton } from "./IconButton";

// ---------------------------------------------------------------------------
// Modal — a titled dialog with a close control. ESC and a scrim click call
// onClose; clicking the panel does not. The overlay, scrim, portal, focus trap,
// and ARIA wiring are `DialogShell`'s, shared with every other dialog.
// ---------------------------------------------------------------------------

type Size = "sm" | "md" | "lg" | "xl";

export interface ModalProps {
  open: boolean;
  onClose: () => void;
  title?: React.ReactNode;
  /** One line under the title, for what the dialog is asking of the reader.
   * A title says which dialog this is; a description says what to do in it. */
  description?: React.ReactNode;
  children: React.ReactNode;
  footer?: React.ReactNode;
  size?: Size;
  /** Prevents dismissal while an operation that must finish is in flight. */
  dismissible?: boolean;
  /** Use `alertdialog` for an urgent decision that blocks other work. */
  role?: DialogRole;
}

const SIZE_MAX_WIDTH: Record<Size, string> = {
  sm: "max-w-sm",
  md: "max-w-lg",
  lg: "max-w-2xl",
  // A table is the content, not an aside to it: `lg` cut a preview of a dozen
  // columns down to four before it scrolled.
  xl: "max-w-[60rem]",
};

export function Modal({
  open,
  onClose,
  title,
  description,
  children,
  footer,
  size = "md",
  dismissible = true,
  role = "dialog",
}: ModalProps) {
  const id = React.useId();
  const titleId = `modal-title-${id}`;
  const descriptionId = `modal-description-${id}`;

  return (
    <DialogShell
      className={cn(
        "surface-card xf-material xf-overlay-pop flex w-full flex-col",
        SIZE_MAX_WIDTH[size],
        "modal-panel",
      )}
      describedBy={description != null ? descriptionId : undefined}
      label="Dialog"
      labelledBy={title != null ? titleId : undefined}
      onDismiss={dismissible ? onClose : undefined}
      open={open}
      role={role}
    >
        {/* The close control lines up with the title when the title is all
            there is, and with the top of the block when a description sits
            under it. Top-aligning a 32px control against a 24px line put the X
            visibly below the words it belongs to. */}
        <div className={cn("flex shrink-0 gap-3 px-5 pt-4 pb-3", description != null ? "items-start" : "items-center")}>
          <div className="min-w-0 flex-1">
            {title != null ? (
              <SectionTitle className="break-words" id={titleId} level={2}>
                {title}
              </SectionTitle>
            ) : null}
            {description != null ? (
              <p id={descriptionId} className="type-caption mt-0.5 text-muted">
                {description}
              </p>
            ) : null}
          </div>
          <IconButton aria-label="Close" disabled={!dismissible} onClick={onClose} className="-mr-1.5">
            <X size={18} aria-hidden />
          </IconButton>
        </div>

        <div className="min-h-0 flex-auto overflow-y-auto overscroll-contain px-5 pb-4 text-sm text-foreground">
          {children}
        </div>

      {footer != null ? (
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 border-t border-hairline px-5 py-3">{footer}</div>
      ) : null}
    </DialogShell>
  );
}
