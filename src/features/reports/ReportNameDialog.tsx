import { type ReactNode, useEffect, useId, useState } from "react";
import { AlertTriangle } from "lucide-react";
import { Button } from "../../ui/Button";
import { Field, Input, Textarea } from "../../ui/Field";
import { Modal } from "../../ui/Modal";

/**
 * "This dashboard is already saved". When present it replaces the confirm
 * button with two explicit choices, so re-saving the same bytes is a deliberate
 * act and never a silent duplicate.
 */
export type DuplicatePrompt = {
  existingTitle: string;
  onOpenExisting: () => void;
  /** Receives the live name and description, so an edited copy is honoured. */
  onSaveCopy: (title: string, description: string | null) => void;
};

export type ReportNameDialogProps = {
  open: boolean;
  heading: string;
  confirmLabel: string;
  initialTitle?: string;
  initialDescription?: string;
  busy?: boolean;
  /** Refusal text from the backend — a cap that was hit, an unusable name. */
  error?: string | null;
  /** The document references something the preview scheme cannot load, so it
   * would render incomplete forever. Shown BEFORE the save, which is the only
   * point at which the reader can still decline. */
  externalReferenceWarning?: boolean;
  duplicate?: DuplicatePrompt | null;
  /**
   * Answers "does another report already carry this name" for the field's live
   * value. Advisory and never blocking: a report is identified by its id, the
   * server has no uniqueness constraint on titles, and saving two reports under
   * one name is a legitimate thing to want. Omitted where the caller has no authoritative
   * list, because a notice that appears only sometimes is worse than none.
   */
  nameInUse?: (title: string) => boolean;
  /** Whether to ask for a description. A report saved as a file in a project
   * has nowhere to keep one, so asking would collect text that is then lost. */
  describe?: boolean;
  /** Where the report will be kept, when that is not the Reports list. */
  destinationNote?: string;
  /** Further fields the caller owns (a project to save into), after the name. */
  children?: ReactNode;
  onConfirm: (title: string, description: string | null) => void;
  onCancel: () => void;
};

/** Name and describe a report — one component for saving and for renaming, as
 * in xfalcon-web. The two differ only in their heading and confirm label; two
 * components would be the same form maintained twice. */
export function ReportNameDialog({
  open,
  heading,
  confirmLabel,
  initialTitle = "",
  initialDescription = "",
  busy = false,
  error = null,
  externalReferenceWarning = false,
  duplicate = null,
  nameInUse,
  describe = true,
  destinationNote,
  children,
  onConfirm,
  onCancel,
}: ReportNameDialogProps) {
  const [title, setTitle] = useState(initialTitle);
  const [description, setDescription] = useState(initialDescription);
  const titleId = useId();
  const descriptionId = useId();
  const nameNoticeId = useId();

  // Reset whenever the dialog reopens, so a previous edit never leaks into the
  // next report.
  useEffect(() => {
    if (!open) return;
    setTitle(initialTitle);
    setDescription(initialDescription);
  }, [open, initialTitle, initialDescription]);

  const trimmed = title.trim();
  const resolvedDescription = !describe || description.trim() === "" ? null : description.trim();
  const titleLength = Array.from(trimmed).length;
  const descriptionLength = resolvedDescription === null ? 0 : Array.from(resolvedDescription).length;
  const titleTooLong = titleLength > 200;
  const descriptionTooLong = descriptionLength > 2000;
  const canConfirm = titleLength > 0 && !titleTooLong && !descriptionTooLong && !busy;
  const nameTaken = nameInUse?.(trimmed) ?? false;

  return (
    <Modal
      open={open}
      size="sm"
      title={heading}
      onClose={() => !busy && onCancel()}
      footer={
        <div className="flex w-full flex-wrap items-center justify-end gap-2 pt-1">
          {duplicate !== null ? (
            <>
              <Button disabled={busy} type="button" variant="secondary" onClick={duplicate.onOpenExisting}>
                Open existing
              </Button>
              <Button
                disabled={!canConfirm}
                loading={busy}
                type="button"
                variant="primary"
                onClick={() => duplicate.onSaveCopy(trimmed, resolvedDescription)}
              >
                Save a copy
              </Button>
            </>
          ) : (
            <>
              <Button disabled={busy} type="button" variant="secondary" onClick={onCancel}>
                Cancel
              </Button>
              <Button disabled={!canConfirm} loading={busy} type="submit" form={`${titleId}-form`} variant="primary">
                {confirmLabel}
              </Button>
            </>
          )}
        </div>
      }
    >
      <form
        id={`${titleId}-form`}
        className="flex flex-col gap-3"
        onSubmit={(event) => {
          event.preventDefault();
          if (!canConfirm) return;
          if (duplicate !== null) {
            duplicate.onSaveCopy(trimmed, resolvedDescription);
          } else {
            onConfirm(trimmed, resolvedDescription);
          }
        }}
      >
        <Field htmlFor={titleId} label="Name" required>
          <Input
            aria-describedby={nameTaken && duplicate === null ? nameNoticeId : undefined}
            autoFocus
            id={titleId}
            placeholder="Report name"
            value={title}
            onChange={(event) => setTitle(event.currentTarget.value)}
          />
        </Field>

        {describe ? (
          <Field htmlFor={descriptionId} hint="Optional" label="Description">
            <Textarea
              id={descriptionId}
              placeholder="What is this report about?"
              rows={3}
              value={description}
              onChange={(event) => setDescription(event.currentTarget.value)}
            />
          </Field>
        ) : null}

        {children}

        {destinationNote ? <p className="type-caption text-muted">{destinationNote}</p> : null}

        {titleTooLong ? (
          <p className="type-caption text-danger" role="alert">
            Name must be 200 characters or fewer.
          </p>
        ) : null}

        {descriptionTooLong ? (
          <p className="type-caption text-danger" role="alert">
            Description must be 2,000 characters or fewer.
          </p>
        ) : null}

        {/* Said, not enforced — and `role="status"` rather than `alert`, because
            it changes on every keystroke and must not interrupt typing. */}
        {nameTaken && duplicate === null ? (
          <p className="type-caption text-muted" id={nameNoticeId} role="status">
            Another report is already called “{trimmed}”. They stay separate reports.
          </p>
        ) : null}

        {externalReferenceWarning && duplicate === null ? (
          <p className="flex items-start gap-2 type-caption text-warning" role="alert">
            <AlertTriangle aria-hidden className="mt-0.5 shrink-0" size={14} strokeWidth={2.2} />
            <span>
              This dashboard loads a script or stylesheet from the internet, which a saved report cannot reach. It will
              open with that part missing.
            </span>
          </p>
        ) : null}

        {error !== null && duplicate === null ? (
          <p className="type-caption text-danger" role="alert">
            {error}
          </p>
        ) : null}

        {duplicate !== null ? (
          <p className="flex items-start gap-2 type-caption text-warning" role="alert">
            <AlertTriangle aria-hidden className="mt-0.5 shrink-0" size={14} strokeWidth={2.2} />
            <span>Already saved as “{duplicate.existingTitle}”. Open it, or save another copy.</span>
          </p>
        ) : null}
      </form>
    </Modal>
  );
}
