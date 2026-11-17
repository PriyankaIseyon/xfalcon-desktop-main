import { useEffect, useRef, useState } from "react";
import { Check, FolderDown } from "lucide-react";
import type { ProjectReportSaveResult } from "../../lib/generated/contracts";
import { reportSaveToProject } from "../../lib/native";
import { toast } from "../../ui/toastStore";
import { ReportNameDialog } from "./ReportNameDialog";
import { type ProjectReportDestination, useProjectReportDestination } from "./reportDestination";

/** Long enough to reach "Open file" from the keyboard; hovering pauses it. */
const ACTIONABLE_TOAST_MS = 8000;

/** What the card knows about the document it is showing. `htmlIndex` is its
 * position among the message's HTML documents, resolved by scanning the
 * message's own text — never a counter incremented as fence components mount,
 * which would be right only by luck. */
export type ReportSource = {
  conversationId: string;
  messageId: string;
  htmlIndex: number;
};

/**
 * "Save to project" on an HTML preview card. It appears only when the card's
 * conversation is bound to a project, and writes the document into that
 * project's folder (spec: reports). A conversation with no project offers no
 * save control at all.
 *
 * Sends an IDENTITY, never the markup: the backend re-reads the document from
 * the persisted message, so this button cannot be used to store bytes the
 * renderer made up. There is deliberately no Download beside it — saving into
 * the project IS the answer to that need, and it costs the app no filesystem
 * authority (spec: reports).
 */
export function SaveAsReportAction({
  source,
  defaultTitle,
  externalReference,
  className,
}: {
  source: ReportSource;
  /** The document's own `<title>`, which is the name a reader recognizes it by. */
  defaultTitle: string;
  /** The document loads something the preview scheme cannot reach, so the
   * saved report would render incomplete. Warned about in the dialog, where
   * declining is still possible — the backend's own check is authoritative and
   * arrives with the result, by which point the bytes are already stored. */
  externalReference: boolean;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [duplicate, setDuplicate] = useState<{ title: string; open: () => void } | null>(null);
  const project = useProjectReportDestination(source.conversationId);
  /** The destination the open dialog was offered for. Held from the moment it
   * opens, so the save goes where the dialog said it would: a binding that
   * drops from the shell's list mid-dialog must fail as a project save. */
  const [target, setTarget] = useState<ProjectReportDestination | null>(null);
  const request = useRef(0);
  const busyRef = useRef(false);
  const sourceKey = `${source.conversationId}\u0000${source.messageId}\u0000${source.htmlIndex}`;
  const currentSourceKey = useRef(sourceKey);
  currentSourceKey.current = sourceKey;

  useEffect(
    () => () => {
      request.current += 1;
      busyRef.current = false;
    },
    [],
  );

  const save = async (title: string, allowDuplicate: boolean) => {
    if (busyRef.current) return;
    if (target === null) {
      setError("This chat is no longer part of a project, so nothing was saved.");
      return;
    }
    busyRef.current = true;
    const currentRequest = ++request.current;
    const requestSourceKey = sourceKey;
    setBusy(true);
    setError(null);
    try {
      const result = await reportSaveToProject(
        source.conversationId,
        source.messageId,
        source.htmlIndex,
        title,
        allowDuplicate,
      );
      if (currentRequest !== request.current || requestSourceKey !== currentSourceKey.current) return;
      settleProject(result, target);
    } catch (cause) {
      if (currentRequest !== request.current || requestSourceKey !== currentSourceKey.current) return;
      setError(refusalText(cause));
    } finally {
      if (currentRequest === request.current) {
        busyRef.current = false;
        setBusy(false);
      }
    }
  };

  const settleProject = (result: ProjectReportSaveResult, destination: ProjectReportDestination) => {
    switch (result.status) {
      case "saved": {
        if (result.file === null) {
          setError("Could not save this report. Try again in a moment.");
          return;
        }
        const path = result.file.relative_path;
        setOpen(false);
        setDuplicate(null);
        setSaved(true);
        toast.success(`Saved to ${destination.projectName} as ${path}`, ACTIONABLE_TOAST_MS, {
          label: "Open file",
          onSelect: () => destination.openFile(path),
        });
        if (result.external_reference) {
          toast.warning("This report references a script it cannot load, so part of it will be missing.");
        }
        return;
      }
      case "duplicate": {
        // A repeat is a choice, not an error: offer the existing file or a
        // deliberate copy rather than a red message.
        if (result.file === null) {
          setError("Could not identify the saved copy. Try again in a moment.");
          return;
        }
        const path = result.file.relative_path;
        setDuplicate({ title: path, open: () => destination.openFile(path) });
        return;
      }
      case "too_large":
        setError(
          result.limit === null
            ? "This dashboard is too large to save into the project."
            : `This dashboard is larger than the ${formatMegabytes(result.limit)} a project report can hold.`,
        );
        return;
      case "not_in_project":
        setError("This chat is no longer part of a project, so nothing was saved.");
        return;
      case "not_found":
      case "not_html":
        setError("This dashboard could not be found in the saved conversation. Reopen the chat and try again.");
        return;
    }
  };

  // Rendered while the dialog is open even if the binding has dropped, so a
  // save started from it still fails as a project save (see `target`).
  if (project === null && !open) return null;
  return (
    <>
      {project !== null ? (
        <button
          // The name starts with the visible words, so a voice command for the
          // visible label finds the button; the rest says which report and where.
          aria-label={`${saved ? "Saved to project" : "Save to project"}: ${defaultTitle} (${project.projectName})`}
          className={className}
          type="button"
          onClick={() => {
            setError(null);
            setDuplicate(null);
            setTarget(project);
            setOpen(true);
          }}
        >
          {saved ? (
            <Check aria-hidden className="badge-pop text-success" size={12} />
          ) : (
            <FolderDown aria-hidden size={12} />
          )}
          {saved ? "Saved to project" : "Save to project"}
        </button>
      ) : null}

      <ReportNameDialog
        busy={busy}
        confirmLabel="Save to project"
        describe={false}
        destinationNote={
          target === null ? undefined : `Saved as an HTML file in the reports folder of ${target.projectName}.`
        }
        duplicate={
          duplicate === null
            ? null
            : {
                existingTitle: duplicate.title,
                onOpenExisting: () => {
                  setOpen(false);
                  setDuplicate(null);
                  duplicate.open();
                },
                onSaveCopy: (title) => {
                  setDuplicate(null);
                  void save(title, true);
                },
              }
        }
        error={error}
        externalReferenceWarning={externalReference}
        heading={target !== null ? `Save to ${target.projectName}` : "Save to project"}
        initialTitle={defaultTitle}
        open={open}
        onCancel={() => !busy && setOpen(false)}
        onConfirm={(title) => void save(title, false)}
      />
    </>
  );
}

/** A rejected save, as a sentence. These are the operation failures — an
 * unusable name, an unreachable folder — as distinct from the outcomes above. */
function refusalText(cause: unknown): string {
  const code =
    typeof cause === "object" && cause !== null && "code" in cause ? String((cause as { code: unknown }).code) : "";
  if (code === "invalid_title") return "Give the report a name of 200 characters or fewer.";
  if (code === "not_found") return "This report is no longer available.";
  if (code === "project_unavailable") {
    return "The project folder could not be reached, so nothing was saved. Check that the folder still exists and try again.";
  }
  return "Could not save this report. Try again in a moment.";
}

/** Whole mebibytes, labelled `MB` — the unit the reader thinks in. */
function formatMegabytes(bytes: bigint | number): string {
  return `${Math.round(Number(bytes) / (1024 * 1024))} MB`;
}
