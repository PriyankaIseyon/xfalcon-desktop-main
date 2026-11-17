import { useContext, useEffect, useRef, useState } from "react";
import { FolderDown } from "lucide-react";
import type { ProjectReportSaveResult, ReportView } from "../../lib/generated/contracts";
import { reportSaveLibraryToProject } from "../../lib/native";
import { Button } from "../../ui/Button";
import { Field } from "../../ui/Field";
import { Select } from "../../ui/Select";
import { toast } from "../../ui/toastStore";
import { ReportNameDialog } from "./ReportNameDialog";
import { ReportProjectsContext } from "./reportDestination";

/** Long enough to reach "Open file" from the keyboard; hovering pauses it. */
const ACTIONABLE_TOAST_MS = 8000;

/**
 * "Save to project" in the report viewer: writes the page on screen, with the rows
 * DA stored for it, into `reports/` of a project the reader picks.
 *
 * It is a snapshot. The file keeps no link to the report and does not refresh, and
 * the dialog says so. It sends identity only (the report, the page, the project
 * and a name): the backend fetches the page from DA itself, so no markup crosses
 * the boundary and nothing the renderer holds can be written in its place.
 *
 * Absent when the reader has no project to copy into.
 */
export function ReportProjectCopy({
  report,
  page,
  disabled,
}: {
  report: ReportView;
  /** The page on screen, or `null` for the report's first. */
  page: string | null;
  disabled: boolean;
}) {
  const choices = useContext(ReportProjectsContext);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [duplicate, setDuplicate] = useState<{ path: string } | null>(null);
  const [projectId, setProjectId] = useState<string | null>(null);
  const request = useRef(0);
  const busyRef = useRef(false);

  useEffect(
    () => () => {
      request.current += 1;
    },
    [],
  );

  if (choices === null || choices.projects.length === 0) return null;
  const selected = choices.projects.find((project) => project.id === projectId) ?? choices.projects[0];

  const begin = () => {
    const preferred = choices.projects.find((project) => project.id === choices.activeProjectId);
    setProjectId((preferred ?? choices.projects[0]).id);
    setError(null);
    setDuplicate(null);
    setOpen(true);
  };

  const save = async (title: string, allowDuplicate: boolean) => {
    if (busyRef.current) return;
    busyRef.current = true;
    const current = ++request.current;
    const target = selected;
    setBusy(true);
    setError(null);
    try {
      const result = await reportSaveLibraryToProject(report.id, page, target.id, title, allowDuplicate);
      if (current !== request.current) return;
      settle(result, target);
    } catch (cause) {
      if (current !== request.current) return;
      setError(refusalText(cause));
    } finally {
      if (current === request.current) {
        busyRef.current = false;
        setBusy(false);
      }
    }
  };

  const settle = (result: ProjectReportSaveResult, target: { id: string; name: string }) => {
    switch (result.status) {
      case "saved": {
        if (result.file === null) {
          setError("Could not save this report. Try again in a moment.");
          return;
        }
        const path = result.file.relative_path;
        setOpen(false);
        setDuplicate(null);
        toast.success(`Saved to ${target.name} as ${path}`, ACTIONABLE_TOAST_MS, {
          label: "Open file",
          onSelect: () => choices.openFile(target.id, path),
        });
        if (result.external_reference) {
          toast.warning("This report references a script it cannot load, so part of it will be missing.");
        }
        return;
      }
      case "duplicate":
        if (result.file === null) {
          setError("Could not identify the saved copy. Try again in a moment.");
          return;
        }
        setDuplicate({ path: result.file.relative_path });
        return;
      case "too_large":
        setError(
          result.limit === null
            ? "This report is too large to save into a project."
            : `This report is larger than the ${Math.round(Number(result.limit) / (1024 * 1024))} MB a project report can hold.`,
        );
        return;
      case "not_in_project":
      case "not_found":
      case "not_html":
        setError("This report could not be saved. Reopen it and try again.");
        return;
    }
  };

  return (
    <>
      <Button
        aria-label={`Save to project: ${report.title}`}
        disabled={disabled}
        type="button"
        variant="secondary"
        onClick={begin}
      >
        <FolderDown aria-hidden size={14} />
        Save to project
      </Button>
      <ReportNameDialog
        busy={busy}
        confirmLabel="Save to project"
        describe={false}
        destinationNote={`A snapshot of this page with the data xFalcon last stored for it, saved as an HTML file in the reports folder of ${selected.name}. It does not refresh.`}
        duplicate={
          duplicate === null
            ? null
            : {
                existingTitle: duplicate.path,
                onOpenExisting: () => {
                  setOpen(false);
                  const path = duplicate.path;
                  setDuplicate(null);
                  choices.openFile(selected.id, path);
                },
                onSaveCopy: (title) => {
                  setDuplicate(null);
                  void save(title, true);
                },
              }
        }
        error={error}
        heading="Save to project"
        initialTitle={report.title}
        open={open}
        onCancel={() => !busy && setOpen(false)}
        onConfirm={(title) => void save(title, false)}
      >
        {choices.projects.length > 1 ? (
          <Field label="Project">
            <Select
              disabled={busy}
              options={choices.projects.map((project) => ({ value: project.id, label: project.name }))}
              value={selected.id}
              onChange={(value) => {
                setProjectId(value);
                // A duplicate found in one project says nothing about another.
                setDuplicate(null);
                setError(null);
              }}
            />
          </Field>
        ) : null}
      </ReportNameDialog>
    </>
  );
}

/** A rejected save, as a sentence: an unusable name, a folder that cannot be reached. */
function refusalText(cause: unknown): string {
  const code =
    typeof cause === "object" && cause !== null && "code" in cause ? String((cause as { code: unknown }).code) : "";
  if (code === "invalid_title") return "Give the report a name of 200 characters or fewer.";
  if (code === "not_found") return "This report is no longer available.";
  if (code === "project_unavailable") {
    return "The project folder could not be reached, so nothing was saved. Check that the folder still exists and try again.";
  }
  if (code === "too_large_to_open") return "This report is too large to save into a project.";
  return "Could not save this report. Try again in a moment.";
}
