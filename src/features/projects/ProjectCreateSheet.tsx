import React from "react";
import { FolderOpen } from "lucide-react";
import type { ProjectView } from "../../lib/generated/contracts";
import { projectActivate, projectCreate, projectFindByFolder, projectPickFolder } from "../../lib/native";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { Field, Input } from "../../ui/Field";
import { Modal } from "../../ui/Modal";
import { describeProjectError } from "./projectFileText";

export interface ProjectCreateSheetProps {
  open: boolean;
  onClose: () => void;
  /** Called after creating a project or activating an existing project. */
  onReady: (project: ProjectView) => void;
}

interface SheetError {
  title: string;
  message: string;
  nextAction: string;
}

/** A folder the person chose: the grant to act with, the text to show for it,
 * and the name the project will get unless they change it. */
interface ChosenFolder {
  grantId: string;
  displayPath: string;
  displayName: string;
}

/** The backend's refusal for a grant that expired or was never issued. */
const EXPIRED_CHOICE = "That choice expired. Choose it again.";

function isExpiredChoice(cause: unknown): boolean {
  const detail = typeof cause === "string" ? cause : cause instanceof Error ? cause.message : "";
  return detail.trim() === EXPIRED_CHOICE;
}

/**
 * Sheet for trusting a new project root. Picking a folder never trusts it by
 * itself -- the user sees exactly what trust grants (scoped read/write
 * access to that folder, including xFalcon writing without asking)
 * before the confirming action calls `projectCreate`.
 */
export function ProjectCreateSheet({ open, onClose, onReady }: ProjectCreateSheetProps) {
  const [chosen, setChosen] = React.useState<ChosenFolder | null>(null);
  const [sharedProjects, setSharedProjects] = React.useState<ProjectView[]>([]);
  const [picking, setPicking] = React.useState(false);
  const [submitting, setSubmitting] = React.useState(false);
  const [error, setError] = React.useState<SheetError | null>(null);
  const requestRef = React.useRef(0);
  const openRef = React.useRef(open);
  openRef.current = open;

  // Reset to a clean first step each time the sheet opens, so a prior
  // failed/cancelled attempt never leaks into the next one.
  React.useEffect(() => {
    requestRef.current += 1;
    if (!open) return;
    setChosen(null);
    setSharedProjects([]);
    setPicking(false);
    setSubmitting(false);
    setError(null);
    return () => {
      requestRef.current += 1;
    };
  }, [open]);

  const choose = async () => {
    if (picking || submitting) return;
    const request = ++requestRef.current;
    setPicking(true);
    try {
      const picked = await projectPickFolder();
      if (request !== requestRef.current || !openRef.current || picked == null) return;
      setError(null);
      setChosen(null);
      setSharedProjects([]);
      try {
        const result = await projectFindByFolder(picked.grant_id);
        if (request !== requestRef.current || !openRef.current) return;
        setSharedProjects(result.projects);
        setChosen({
          grantId: picked.grant_id,
          displayPath: picked.display_path,
          displayName: picked.display_name,
        });
      } catch (cause: unknown) {
        if (request === requestRef.current && openRef.current) {
          setError({
            title: "The folder could not be checked",
            message: describeProjectError(cause, "Existing projects could not be checked."),
            nextAction: isExpiredChoice(cause) ? "Choose the folder again." : "Choose the folder again to retry.",
          });
        }
      }
    } catch {
      if (request === requestRef.current && openRef.current) {
        setError({
          title: "The folder picker did not open",
          message: "xFalcon could not open the system folder picker.",
          nextAction: "Try again.",
        });
      }
    } finally {
      if (request === requestRef.current && openRef.current) setPicking(false);
    }
  };

  const openExisting = async (projectId: string) => {
    if (picking || submitting) return;
    const request = ++requestRef.current;
    setSubmitting(true);
    setError(null);
    try {
      const project = await projectActivate(projectId);
      if (request !== requestRef.current || !openRef.current) return;
      setSubmitting(false);
      requestRef.current += 1;
      onReady(project);
      onClose();
    } catch (cause: unknown) {
      if (request === requestRef.current && openRef.current) {
        setError({
          title: "The project could not be opened",
          message: describeProjectError(cause, "This project is unavailable."),
          nextAction: "Try again or choose a different folder.",
        });
      }
    } finally {
      if (request === requestRef.current && openRef.current) setSubmitting(false);
    }
  };

  const confirm = async () => {
    if (chosen == null || picking || submitting) return;
    const request = ++requestRef.current;
    setSubmitting(true);
    setError(null);
    try {
      const project = await projectCreate(chosen.grantId, chosen.displayName.trim() || undefined);
      if (request !== requestRef.current || !openRef.current) return;
      setSubmitting(false);
      requestRef.current += 1;
      onReady(project);
      onClose();
    } catch (cause: unknown) {
      if (request === requestRef.current && openRef.current) {
        // An expired choice cannot be retried: send the person back to the
        // picker instead of leaving a confirm button that fails the same way.
        if (isExpiredChoice(cause)) setChosen(null);
        setError({
          title: "The project was not added",
          message: describeProjectError(cause, "This project could not be added."),
          nextAction: isExpiredChoice(cause)
            ? "Choose the folder again."
            : "Check that the folder is still available, then try again.",
        });
      }
    } finally {
      if (request === requestRef.current && openRef.current) setSubmitting(false);
    }
  };

  const close = () => {
    if (picking || submitting) return;
    requestRef.current += 1;
    onClose();
  };

  return (
    <Modal
      footer={
        chosen == null ? (
          <Button disabled={picking} onClick={close} variant="secondary">
            Cancel
          </Button>
        ) : (
          <>
            <Button disabled={submitting} loading={picking} onClick={() => void choose()} variant="secondary">
              Choose a different folder
            </Button>
            <Button disabled={picking} loading={submitting} onClick={() => void confirm()} variant="primary">
              Trust &amp; create project
            </Button>
          </>
        )
      }
      dismissible={!picking && !submitting}
      onClose={close}
      open={open}
      title="New project"
    >
      {chosen == null ? (
        <div className="flex flex-col gap-3">
          <p className="type-body text-secondary">Choose a local folder to use as a project.</p>
          <Button className="w-fit" loading={picking} onClick={() => void choose()} variant="primary">
            <FolderOpen aria-hidden size={16} strokeWidth={1.8} />
            Choose folder
          </Button>
        </div>
      ) : (
        <div className="flex flex-col gap-4">
          <Field htmlFor="project-create-name" label="Project name">
            <Input
              disabled={submitting}
              id="project-create-name"
              onChange={(event) => {
                const name = event.target.value;
                setChosen((current) => (current == null ? current : { ...current, displayName: name }));
              }}
              value={chosen.displayName}
            />
          </Field>
          <p className="type-caption break-all text-muted">{chosen.displayPath}</p>
          {sharedProjects.length > 0 ? (
            <Alert title="Shared project folder" tone="warn">
              <p>
                This folder is already used by{" "}
                {sharedProjects.map((project, index) => (
                  <React.Fragment key={project.id}>
                    {index > 0 ? ", " : ""}
                    <button
                      type="button"
                      className="break-words font-semibold underline underline-offset-2 disabled:opacity-50"
                      aria-label={`Open ${project.display_name}`}
                      disabled={picking || submitting}
                      onClick={() => void openExisting(project.id)}
                    >
                      {project.display_name}
                    </button>
                  </React.Fragment>
                ))}
                .
              </p>
              <p className="mt-1">File edits in any project affect all projects using this folder.</p>
              <p className="mt-1">
                Choose a different folder or open an existing project. You can also continue with this folder.
              </p>
            </Alert>
          ) : null}
          <Alert title="Trusting this folder" tone="info">
            <ul className="list-disc space-y-1 pl-4">
              <li>
                xFalcon can read any file in this folder, and creates, updates, moves, and deletes files here
                automatically when your request calls for it, without asking first. Trust grants scoped access to this
                folder only.
              </li>
              <li>Before a file is changed or deleted, its earlier version is kept under History &amp; Revisions.</li>
              <li>Changes you make in the file editor are written only when you select Save.</li>
            </ul>
          </Alert>
          <p className="type-caption text-secondary">
            When xFalcon detects permanent deletion of this folder, it removes the project and its chats.
          </p>
        </div>
      )}
      {error ? (
        <Alert className="mt-4" title={error.title} tone="danger">
          <p>{error.message}</p>
          <p className="mt-1">{error.nextAction}</p>
        </Alert>
      ) : null}
    </Modal>
  );
}
