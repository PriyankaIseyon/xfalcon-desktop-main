import React from "react";
import { FolderOpen, FileText, Upload } from "lucide-react";
import type { SkillImportFailure, SkillPickKind, SkillSourceGrantView } from "../../lib/generated/contracts";
import { onSkillSources, skillImport, skillPick } from "../../lib/native";
import { Alert } from "../../ui/Alert";
import { Modal } from "../../ui/Modal";
import { Spinner } from "../../ui/Spinner";
import { toast } from "../../ui/toastStore";
import { SettingsButton } from "./SettingsControls";
import { SKILLS_COPY, SKILL_IMPORT_COPY, describeSkillError } from "./settingsCopy";

export interface SkillImportDialogProps {
  open: boolean;
  onClose: () => void;
  /** Called after an import that stored at least one skill, so the library
   * behind the dialog re-reads itself rather than trusting this component's
   * word for what is now on disk. */
  onImported: () => void;
}

/**
 * The one way skills get in (spec: user-skills).
 *
 * There is a single entry point rather than one button per input kind: a file,
 * an archive, and a folder are three spellings of "bring me your skills", and a
 * row of buttons that each open a different picker makes the reader choose
 * between them before they have been told what the difference is. The dialog
 * explains the rules once and then offers the inputs.
 *
 * Dropping is the primary path because it is the only one that takes files,
 * archives, and folders at the same time. The two pickers stay for people who
 * do not drag, and because a system dialog cannot offer both kinds at once.
 *
 * Grants cross the boundary, never paths or bytes -- see `onSkillSources`.
 */
export function SkillImportDialog({ open, onClose, onImported }: SkillImportDialogProps) {
  const [importing, setImporting] = React.useState(false);
  const [dragging, setDragging] = React.useState(false);
  const [failures, setFailures] = React.useState<SkillImportFailure[]>([]);
  const importingRef = React.useRef(false);

  // Held in a ref so the drop listener, registered once per open, always calls
  // the current import rather than the one captured when it was registered.
  const importGrantRef = React.useRef<(grantId: string) => void>(() => {});

  const runImport = React.useCallback(
    async (grantId: string) => {
      if (importingRef.current) return;
      importingRef.current = true;
      setImporting(true);
      setFailures([]);
      try {
        const result = await skillImport(grantId);
        setFailures(result.failed);
        if (result.imported.length > 0) {
          toast.success(
            result.imported.length === 1
              ? `Imported “${result.imported[0].name}”.`
              : `Imported ${result.imported.length} skills.`,
          );
          onImported();
        }
        // Failures keep the dialog open: they are the answer to what was just
        // attempted, and closing over them would throw the reasons away.
        if (result.failed.length === 0) onClose();
      } catch (cause: unknown) {
        toast.error(describeSkillError(cause, SKILLS_COPY.importFailed));
      } finally {
        importingRef.current = false;
        setImporting(false);
      }
    },
    [onClose, onImported],
  );

  importGrantRef.current = (grantId: string) => void runImport(grantId);

  // Only listen while the dialog is open: the events are webview-wide, and a
  // file dropped on the app at any other moment must not import itself.
  React.useEffect(() => {
    if (!open) return;
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    void onSkillSources((event) => {
      if (event.kind === "rejected") {
        setDragging(false);
        toast.error(describeSkillError({ code: event.code }, SKILLS_COPY.importFailed));
        return;
      }
      if (event.kind === "dropped") {
        setDragging(false);
        if (importingRef.current) {
          toast.message("Wait for the current import to finish, then drop the files again.");
        } else {
          importGrantRef.current(event.grant.grant_id);
        }
        return;
      }
      setDragging(event.kind === "over");
    })
      .then((stop) => {
        // Resolved after the dialog already closed -- stop immediately rather
        // than leaving a listener behind.
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => {
        // A window that cannot report drops still has both pickers; failing
        // loudly here would be an error about a path the user has not taken.
      });

    return () => {
      cancelled = true;
      unlisten?.();
      setDragging(false);
    };
  }, [open]);

  React.useEffect(() => {
    if (!open) {
      setFailures([]);
      importingRef.current = false;
    }
  }, [open]);

  const choose = async (kind: SkillPickKind) => {
    if (importing) return;
    let grant: SkillSourceGrantView | null;
    try {
      grant = await skillPick(kind);
    } catch (cause: unknown) {
      toast.error(describeSkillError(cause, "The system picker could not be opened."));
      return;
    }
    // `null` is the user cancelling the picker, which is not an error.
    if (grant === null) return;
    // A drop may have started an import while the picker was open. Say so,
    // rather than letting the chosen files vanish without a word.
    if (importingRef.current) {
      toast.message("Wait for the current import to finish, then choose the files again.");
      return;
    }
    await runImport(grant.grant_id);
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      dismissible={!importing}
      size="lg"
      title={SKILL_IMPORT_COPY.title}
      footer={
        <div className="flex w-full flex-wrap items-center justify-between gap-3">
          <p className="type-caption text-muted">{SKILL_IMPORT_COPY.replaceNote}</p>
          <SettingsButton className="shrink-0" disabled={importing} onClick={onClose}>
            {SKILL_IMPORT_COPY.doneLabel}
          </SettingsButton>
        </div>
      }
    >
      <div className="flex flex-col gap-4">
        <p className="type-body text-secondary">{SKILL_IMPORT_COPY.howItWorks}</p>

        <div>
          <p className="type-label font-semibold text-foreground">{SKILL_IMPORT_COPY.acceptsTitle}</p>
          <ul className="type-caption mt-1 flex flex-col gap-1 text-secondary">
            {SKILL_IMPORT_COPY.accepts.map((line) => (
              <li key={line} className="flex gap-2">
                <span aria-hidden className="text-muted">
                  •
                </span>
                <span>{line}</span>
              </li>
            ))}
          </ul>
        </div>

        {/* Not a button: dropping is a pointer gesture, and the pickers below
            are the keyboard-reachable way to the same result. Kept readable
            rather than `aria-hidden` — it names what the dialog accepts, and
            it is where the import's progress is announced. */}
        <div
          className={
            dragging
              ? "flex flex-col items-center gap-2 rounded-xl border-2 border-dashed border-accent bg-accent/10 px-4 py-8 text-center"
              : "flex flex-col items-center gap-2 rounded-xl border-2 border-dashed border-default px-4 py-8 text-center"
          }
        >
          {importing ? <Spinner decorative size={20} /> : <Upload aria-hidden size={20} strokeWidth={1.8} />}
          <p aria-live="polite" className="type-label font-semibold text-foreground" role="status">
            {importing
              ? SKILL_IMPORT_COPY.importingLabel
              : dragging
                ? SKILL_IMPORT_COPY.dropActive
                : SKILL_IMPORT_COPY.dropTitle}
          </p>
          <p className="type-caption text-secondary">{SKILL_IMPORT_COPY.dropHint}</p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <span className="type-caption text-muted">{SKILL_IMPORT_COPY.orLabel}</span>
          <SettingsButton disabled={importing} icon={<FileText />} onClick={() => void choose("files")}>
            {SKILL_IMPORT_COPY.chooseFilesLabel}
          </SettingsButton>
          <SettingsButton disabled={importing} icon={<FolderOpen />} onClick={() => void choose("folder")}>
            {SKILL_IMPORT_COPY.chooseFolderLabel}
          </SettingsButton>
        </div>
        <p className="type-caption text-muted">{SKILL_IMPORT_COPY.chooserNote}</p>

        {failures.length > 0 ? (
          <Alert title={SKILL_IMPORT_COPY.failuresTitle} tone="warn">
            <ul className="type-caption flex flex-col gap-1">
              {failures.map((failure, index) => (
                <li key={`${failure.path}-${index}`}>
                  <span className="break-all font-medium text-foreground">{failure.path}</span> — {failure.reason}
                </li>
              ))}
            </ul>
            <p className="type-caption mt-2 text-secondary">{SKILL_IMPORT_COPY.foundNote}</p>
          </Alert>
        ) : null}
      </div>
    </Modal>
  );
}
