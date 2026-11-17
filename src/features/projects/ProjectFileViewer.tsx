import React from "react";
import { History, Pencil, RotateCcw, Save, X } from "lucide-react";
import { formatBytes } from "../../lib/formatBytes";
import type {
  ProjectDocumentRead,
  ProjectFileRevisionDiff,
  ProjectFileRevisionView,
  ProjectFileStat,
} from "../../lib/generated/contracts";
import {
  projectFileRestore,
  projectFileRevisionDiff,
  projectFileRevisions,
  projectFileStat,
  projectSaveFile,
} from "../../lib/native";
import { Alert } from "../../ui/Alert";
import { AlertDialog } from "../../ui/AlertDialog";
import { Button } from "../../ui/Button";
import { Textarea } from "../../ui/Field";
import { Label } from "../../ui/Label";
import { SegmentedControl } from "../../ui/SegmentedControl";
import { Spinner } from "../../ui/Spinner";
import { Tabs } from "../../ui/Tabs";
import { LargeTableView } from "./LargeTableView";
import { PROJECT_FILE_LIMITS } from "./projectFileLimits";
import { type LoadedFile, isPreviewableOfficePath, loadProjectFile, sameVersion } from "./projectFileLoad";
import { ProjectOfficeViewer } from "./ProjectOfficeViewer";
import { projectFileRendering } from "./projectFileRendering";
import { FileActionsRow, FileLines, ProjectFileRichView } from "./ProjectFileRichView";
import {
  METADATA_ONLY_REASON,
  SENSITIVE_CLASS_LABEL,
  describeProjectError,
  inOriginalLineEndings,
  utf8ToBase64,
} from "./projectFileText";
import { WindowedTextView } from "./WindowedTextView";
import { EmptyState } from "../../ui/EmptyState";
import { FileDiffViewer } from "./FileDiffViewer";
import { ProjectSchemaViewer } from "./ProjectSchemaViewer";

type ViewState = { status: "loading" } | { status: "loaded"; file: LoadedFile } | { status: "error"; message: string };

/** One file that is open in the view. */
interface OpenFileProps {
  projectId: string;
  path: string;
  drafts?: UnsavedDraftStore;
}

/**
 * Unsaved edits that outlive the view that held them, keyed by project and path.
 *
 * Opening another file, or another tab, unmounts the editor and with it the
 * only copy of what was typed. The owner of the view supplies one of these so a
 * draft is put back when its file is opened again; without one nothing is kept,
 * which is what a view rendered on its own (and its tests) wants.
 */
export type UnsavedDraftStore = Map<string, { text: string; baseline: ProjectFileStat }>;

/** How often the open file is checked against disk while the window is showing. */
const STALE_POLL_MS = 3_000;

/** Drafts kept at once. Each is at most one editable file's text. */
const MAX_UNSAVED_DRAFTS = 20;

export interface ProjectFileViewerProps {
  projectId: string;
  /** Project-relative path of the file to show; `null` before one is picked. */
  path: string | null;
  drafts?: UnsavedDraftStore;
}

/** What the save button is doing right now. */
type SaveState =
  { status: "idle" } | { status: "saving" } | { status: "saved" } | { status: "failed"; message: string };

function ProjectFileContents({ projectId, path, drafts }: OpenFileProps) {
  const [state, setState] = React.useState<ViewState>({ status: "loading" });
  const [reloadToken, setReloadToken] = React.useState(0);
  const [draft, setDraft] = React.useState<string | null>(null);
  const [save, setSave] = React.useState<SaveState>({ status: "idle" });
  const [staleOnDisk, setStaleOnDisk] = React.useState(false);
  const [confirmReload, setConfirmReload] = React.useState(false);
  const [restoredDraft, setRestoredDraft] = React.useState(false);
  const draftKey = `${projectId}:${path}`;
  /** What the editor held at the last commit, for the moment it unmounts. */
  const liveRef = React.useRef<{ draft: string | null; text: string | null }>({ draft: null, text: null });
  /** The editor's text right now, for the timer, which must not re-arm on every keystroke. */
  const draftRef = React.useRef<string | null>(null);
  const generationRef = React.useRef(0);
  const staleProbeRef = React.useRef(0);
  /** The stat the content on screen was read at. */
  const baselineRef = React.useRef<ProjectFileStat | ProjectDocumentRead | null>(null);
  const rendering = React.useMemo(() => projectFileRendering(path), [path]);
  // A table too large to parse here is paged by the backend, so it is never
  // loaded whole: the whole-load bound for it is the in-memory table bound.
  const wholeLimit =
    rendering.kind === "delimited" ? PROJECT_FILE_LIMITS.inMemoryTableBytes : PROJECT_FILE_LIMITS.wholeLoadBytes;

  React.useEffect(() => {
    const generation = ++generationRef.current;
    staleProbeRef.current += 1;
    baselineRef.current = null;
    setState({ status: "loading" });
    setDraft(null);
    setSave({ status: "idle" });
    setStaleOnDisk(false);
    setConfirmReload(false);
    setRestoredDraft(false);
    // Aborted on a switch or reload, so a superseded load stops reading
    // rather than finishing tens of megabytes only to be discarded.
    const controller = new AbortController();
    loadProjectFile(projectId, path, wholeLimit, controller.signal).then(
      (file) => {
        if (generation !== generationRef.current) return;
        baselineRef.current = file.stat;
        setState({ status: "loaded", file });
        // Edits left behind when this file was last open come back, but only
        // onto the very version they were made against: onto a file that has
        // changed since, they would overwrite what changed.
        const remembered = drafts?.get(draftKey);
        drafts?.delete(draftKey);
        if (remembered !== undefined && file.mode === "whole" && sameVersion(remembered.baseline, file.stat)) {
          setDraft(remembered.text);
          setRestoredDraft(true);
        }
      },
      (cause: unknown) => {
        if (generation === generationRef.current) {
          setState({ status: "error", message: describeProjectError(cause, "This file could not be read.") });
        }
      },
    );
    return () => {
      controller.abort();
      if (generation === generationRef.current) generationRef.current += 1;
      staleProbeRef.current += 1;
    };
  }, [projectId, path, reloadToken, wholeLimit, drafts, draftKey]);

  // Keeps the editor's unsaved text when the view goes away.
  React.useEffect(() => {
    draftRef.current = draft;
    liveRef.current = {
      draft,
      text: state.status === "loaded" && state.file.mode === "whole" ? state.file.text : null,
    };
  });
  React.useEffect(
    () => () => {
      const { draft: live, text: original } = liveRef.current;
      const baseline = baselineRef.current;
      if (drafts === undefined || live === null || original === null || baseline === null || !("kind" in baseline))
        return;
      if (inOriginalLineEndings(live, original) === original) return;
      drafts.delete(draftKey);
      drafts.set(draftKey, { text: live, baseline });
      while (drafts.size > MAX_UNSAVED_DRAFTS) {
        const oldest = drafts.keys().next().value;
        if (oldest === undefined) break;
        drafts.delete(oldest);
      }
    },
    [drafts, draftKey],
  );

  /*
   * Something outside this app may have changed the file while it sat on
   * screen. A stat on focus notices that without a filesystem watcher and
   * without re-reading the file, and it deliberately does not swap the
   * content: an unsaved draft must never be replaced under the person typing
   * it, so this only offers a reload.
   */
  React.useEffect(() => {
    const check = (quiet: boolean) => {
      if (document.visibilityState !== "visible") return;
      const baseline = baselineRef.current;
      if (baseline == null) return;
      const generation = generationRef.current;
      const probe = ++staleProbeRef.current;
      projectFileStat(projectId, path).then(
        (stat) => {
          if (generation === generationRef.current && probe === staleProbeRef.current) {
            const changed = !sameVersion(baseline, stat);
            // A quiet check is the timer's. When nobody is editing, the newer
            // file simply replaces the older one on screen, which is what
            // someone watching an agent work on it wants; with edits open it
            // falls back to the banner.
            if (changed && quiet && draftRef.current === null) setReloadToken((token) => token + 1);
            else setStaleOnDisk(changed);
          }
        },
        // A file that has since become unreadable is not a staleness signal;
        // the next explicit read will report it properly.
        () => {},
      );
    };
    const onFocus = () => check(false);
    // A chain of timeouts rather than an interval: the frontend boundary test
    // forbids `setInterval`, and a chain cannot pile up behind a slow stat.
    let timer = 0;
    const tick = () => {
      check(true);
      timer = window.setTimeout(tick, STALE_POLL_MS);
    };
    timer = window.setTimeout(tick, STALE_POLL_MS);
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onFocus);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onFocus);
    };
  }, [projectId, path]);

  // Once per text, not on every render: the line-ending check reads the whole file.
  const loadedText = state.status === "loaded" && state.file.mode === "whole" ? state.file.text : null;
  const dirty = React.useMemo(
    () => draft != null && loadedText !== null && inOriginalLineEndings(draft, loadedText) !== loadedText,
    [draft, loadedText],
  );

  if (state.status === "loading") {
    return (
      <p className="type-body flex items-center gap-2 p-4 text-secondary">
        <Spinner decorative size={16} />
        Reading {path}…
      </p>
    );
  }

  if (state.status === "error") {
    return (
      <div className="p-4">
        <Alert title="This file could not be read" tone="danger">
          <p>{state.message}</p>
          <Button className="mt-2" onClick={() => setReloadToken((token) => token + 1)} size="sm" variant="secondary">
            Try again
          </Button>
        </Alert>
      </div>
    );
  }

  const { file } = state;

  if (file.mode === "opaque") {
    return <OpaqueFile stat={file.stat} />;
  }

  if (file.mode === "windowed") {
    return (
      <div className="flex min-h-0 flex-1 flex-col gap-3 p-4">
        {rendering.kind === "delimited" ? (
          <LargeTableView delimiter={rendering.delimiter} path={path} projectId={projectId} />
        ) : (
          <WindowedTextView path={path} projectId={projectId} size={file.stat.size_bytes} />
        )}
      </div>
    );
  }

  const text = file.text;
  const editing = draft != null;
  const editable = file.stat.size_bytes <= PROJECT_FILE_LIMITS.editBytes;
  // The editor reports bare `\n`; compared and saved in the file's own endings,
  // so typing and then undoing is not a change and saving does not rewrite every
  // line break (see `dirty` above).
  const saving = save.status === "saving";
  // File-level actions. They join the card's own action row (Copy, Show
  // source, …) when the file renders as a card, and form one row of their own
  // when it renders as plain lines.
  const fileActions = (
    <>
      {/* "Reload file", not bare "Reload": the tree pane beside this view has
          its own reload, and two unqualified "Reload"s on one screen would
          read as duplicates rather than different scopes. */}
      <Button onClick={() => requestReload()} size="sm" variant="secondary">
        <RotateCcw aria-hidden size={14} strokeWidth={1.8} />
        Reload file
      </Button>
      <Button
        disabled={!editable}
        onClick={() => {
          setDraft(text);
          setSave({ status: "idle" });
        }}
        size="sm"
        variant="secondary"
      >
        <Pencil aria-hidden size={14} strokeWidth={1.8} />
        Edit
      </Button>
    </>
  );

  const requestReload = () => {
    if (dirty) setConfirmReload(true);
    else setReloadToken((token) => token + 1);
  };

  /**
   * Writes the draft and leaves edit mode.
   *
   * The saved text becomes what the viewer shows, without re-reading: the
   * whole file is on screen, so the bytes just written ARE the file.
   *
   * A failed save keeps the draft, because the bytes it holds are the only copy.
   */
  const saveDraft = async () => {
    if (draft == null || saving || staleOnDisk) return;
    const saved = inOriginalLineEndings(draft, text);
    const generation = generationRef.current;
    staleProbeRef.current += 1;
    setSave({ status: "saving" });
    try {
      // Focus checks are advisory. Stat again at the save boundary so an
      // external edit is not overwritten merely because the window never lost
      // focus.
      const onDisk = await projectFileStat(projectId, path);
      if (generation !== generationRef.current) return;
      if (onDisk.path !== path) throw new Error("the safety check returned a different file");
      const baseline = baselineRef.current;
      if (baseline == null || !sameVersion(baseline, onDisk)) {
        setStaleOnDisk(true);
        setSave({ status: "idle" });
        return;
      }

      await projectSaveFile(projectId, path, utf8ToBase64(saved));
      if (generation !== generationRef.current) return;
      const written = await projectFileStat(projectId, path);
      if (generation !== generationRef.current) return;
      staleProbeRef.current += 1;
      setDraft(null);
      setSave({ status: "saved" });
      setStaleOnDisk(false);
      if (written.kind === "text") {
        baselineRef.current = written;
        setState({ status: "loaded", file: { mode: "whole", stat: written, text: saved } });
      } else {
        // The file is no longer one this view shows as text; read it again
        // rather than show the draft as if it were.
        setReloadToken((token) => token + 1);
      }
    } catch (cause: unknown) {
      if (generation !== generationRef.current) return;
      setSave({
        status: "failed",
        message: describeProjectError(cause, "This file could not be saved."),
      });
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 p-4">
      {editing ? (
        <FileActionsRow>
          <Button disabled={saving} onClick={() => setDraft(null)} size="sm" variant="secondary">
            <X aria-hidden size={14} strokeWidth={1.8} />
            Discard changes
          </Button>
          <Button
            disabled={!dirty || saving || staleOnDisk}
            onClick={() => void saveDraft()}
            size="sm"
            variant="primary"
          >
            {saving ? <Spinner decorative size={14} /> : <Save aria-hidden size={14} strokeWidth={1.8} />}
            {saving ? "Saving…" : "Save"}
          </Button>
        </FileActionsRow>
      ) : null}

      {editable ? null : (
        <p className="type-caption text-muted">
          This file is larger than the {formatBytes(PROJECT_FILE_LIMITS.editBytes)} the editor holds, so it is read-only
          here.
        </p>
      )}

      <div aria-live="polite">
        {staleOnDisk ? (
          <Alert title="This file changed on disk" tone="warn">
            <p>
              Something outside xFalcon edited {path} after it was read here. What you see is the older version.
              {editing
                ? " Save is unavailable to protect the newer file. Reload the file before you edit it again."
                : ""}
            </p>
            <Button className="mt-2" disabled={saving} onClick={requestReload} size="sm" variant="secondary">
              <RotateCcw aria-hidden size={14} strokeWidth={1.8} />
              Reload file
            </Button>
          </Alert>
        ) : null}
        {restoredDraft && editing ? (
          <Alert title="Your unsaved changes are back" tone="info">
            <p>You left {path} with edits that were never saved. They are in the editor again.</p>
          </Alert>
        ) : null}
        {save.status === "saved" ? <Alert title={`Saved ${path}`} tone="success" /> : null}
        {save.status === "failed" ? (
          <Alert title="This file was not saved" tone="danger">
            <p>{save.message}</p>
            <p className="mt-1">Unsaved changes are still here, so you can try again.</p>
          </Alert>
        ) : null}
      </div>

      {editing ? (
        <>
          <Label htmlFor="project-file-editor">File contents</Label>
          <Textarea
            className="min-h-64 flex-1 font-mono text-xs leading-relaxed"
            id="project-file-editor"
            disabled={saving}
            onChange={(event) => {
              setDraft(event.target.value);
              setSave({ status: "idle" });
            }}
            spellCheck={false}
            value={draft}
          />
        </>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col gap-2">
          {/*
           * Rendered through the chat's own cards when the type has one, and
           * as text-node lines otherwise. The cards carry the chat's isolation
           * with them -- sandboxed preview origin, sanitizer, text-node cells
           * -- so a crafted file has nothing new to inject into.
           */}
          {rendering.kind === "plain" ? (
            <>
              <FileActionsRow>{fileActions}</FileActionsRow>
              <FileLines path={path} text={text} />
            </>
          ) : (
            <ProjectFileRichView
              actions={fileActions}
              path={path}
              projectId={projectId}
              rendering={rendering}
              sizeBytes={file.stat.size_bytes}
              text={text}
            />
          )}
        </div>
      )}
      <AlertDialog
        confirmLabel="Discard and reload"
        description={`This discards unsaved changes to ${path} and reads the newer file from disk.`}
        destructive
        onCancel={() => setConfirmReload(false)}
        onConfirm={() => {
          setConfirmReload(false);
          setReloadToken((token) => token + 1);
        }}
        open={confirmReload}
        title="Discard unsaved changes?"
      />
    </div>
  );
}

/** A file this view does not show as text, and why. */
function OpaqueFile({ stat }: { stat: ProjectFileStat }) {
  switch (stat.kind) {
    case "sensitive_withheld":
      return (
        <div className="p-4">
          <Alert title="This file was not read" tone="warn">
            <p>
              xFalcon classifies this file as sensitive ({SENSITIVE_CLASS_LABEL[stat.sensitive_class]}) and did not open
              it. That decision is made from the path alone, before the file is touched, so nothing about its contents —
              not even its size — was read.
            </p>
            <p className="mt-1">The file itself is intact. It is withheld, not empty and not damaged.</p>
          </Alert>
        </div>
      );
    case "too_large":
      return (
        <div className="flex flex-col gap-3 p-4">
          <Alert title="Too large to open here" tone="info">
            <p>
              This file is {formatBytes(stat.size_bytes)}, larger than the {formatBytes(stat.limit_bytes)} this view
              opens. It was not read.
            </p>
          </Alert>
        </div>
      );
    case "not_text":
      return (
        <div className="flex flex-col gap-3 p-4">
          <Alert title="Contents not shown" tone="info">
            <p>{METADATA_ONLY_REASON[stat.reason]}</p>
          </Alert>
          <dl className="type-caption grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-muted">
            <dt>Size</dt>
            <dd className="text-secondary">{formatBytes(stat.size_bytes)}</dd>
          </dl>
        </div>
      );
    case "text":
      return null;
  }
}

type RevisionDiffState =
  { status: "loading" } | { status: "loaded"; diff: ProjectFileRevisionDiff } | { status: "error"; message: string };

const REVISION_DIFF_KINDS = new Set<ProjectFileRevisionDiff["kind"]>([
  "changes",
  "unchanged",
  "not_text",
  "too_large",
  "sensitive_withheld",
]);

type RevisionCompare = "current" | "next";

/**
 * What changed from one revision, loaded only when the reader asks for it:
 * most history is never opened, and each diff decrypts a stored copy. Side by
 * side, the revision on the left.
 *
 * On the right, by default, the file as it is now -- the question before a
 * restore is what it would undo. A revision with a newer one after it can
 * instead be compared with that one, which is what the single edit that
 * replaced it changed.
 */
function RevisionChanges({
  projectId,
  path,
  backupId,
  next,
  id,
}: OpenFileProps & { backupId: string; next: ProjectFileRevisionView | null; id: string }) {
  const [state, setState] = React.useState<RevisionDiffState>({ status: "loading" });
  const [attempt, setAttempt] = React.useState(0);
  const [compare, setCompare] = React.useState<RevisionCompare>("current");
  const againstId = compare === "next" && next ? next.backup_id : null;

  React.useEffect(() => {
    let active = true;
    setState({ status: "loading" });
    projectFileRevisionDiff(projectId, path, backupId, againstId).then(
      (diff) => {
        if (!active) return;
        if (diff.version !== 1 || diff.backup_id !== backupId || !REVISION_DIFF_KINDS.has(diff.kind)) {
          setState({ status: "error", message: "The diff returned invalid data. Try again." });
          return;
        }
        setState({ status: "loaded", diff });
      },
      (cause: unknown) => {
        if (active)
          setState({ status: "error", message: describeProjectError(cause, "The diff could not be loaded.") });
      },
    );
    return () => {
      active = false;
    };
  }, [againstId, attempt, backupId, path, projectId]);

  const right = againstId && next ? `the ${formatRevisionDate(next.created_at_ms)} revision` : "the file now";

  let body: React.ReactNode;
  if (state.status === "loading") {
    body = (
      <p className="type-caption flex items-center gap-2 text-secondary">
        <Spinner decorative size={14} /> Loading changes…
      </p>
    );
  } else if (state.status === "error") {
    body = (
      <Alert
        action={
          <Button onClick={() => setAttempt((value) => value + 1)} size="sm" variant="secondary">
            Try again
          </Button>
        }
        title="Changes were not loaded"
        tone="danger"
      >
        {state.message}
      </Alert>
    );
  } else {
    const { diff } = state;
    switch (diff.kind) {
      case "changes":
        body = (
          <>
            <p className="type-caption text-muted">
              {`${diff.added_lines} added · ${diff.removed_lines} removed. Left: this revision. Right: ${right}.`}
            </p>
            <FileDiffViewer defaultMode="split" source={diff.unified_diff} />
          </>
        );
        break;
      case "unchanged":
        body = (
          <p className="type-caption text-secondary">
            {againstId ? "The next revision is the same as this one." : "The file is the same as this revision."}
          </p>
        );
        break;
      case "not_text":
        body = (
          <Alert title="No line diff for this file" tone="info">
            This revision or the current file is not plain text, so there are no lines to compare.
          </Alert>
        );
        break;
      case "too_large":
        body = (
          <Alert title="Diff too large to show" tone="warn">
            {diff.limit === "file_size"
              ? "This revision or the current file is too large to compare here."
              : "This edit changed too much to show as a diff."}{" "}
            You can still restore the revision.
          </Alert>
        );
        break;
      case "sensitive_withheld":
        body = (
          <Alert title="Changes withheld" tone="info">
            {`This path is classified as ${SENSITIVE_CLASS_LABEL[diff.sensitive_class]}, so xFalcon does not read it to compare.`}
          </Alert>
        );
        break;
    }
  }

  return (
    <div
      aria-label={againstId ? "The edit that replaced this revision" : "Changes since this revision"}
      className="flex w-full flex-col gap-2 pt-1"
      id={id}
      role="region"
    >
      {next ? (
        <SegmentedControl
          aria-label="Compare this revision with"
          className="self-start"
          onChange={setCompare}
          options={[
            { value: "current", label: "Since this revision" },
            { value: "next", label: "This change only" },
          ]}
          value={compare}
        />
      ) : null}
      {body}
    </div>
  );
}

function RevisionsPanel({ projectId, path, onRestored }: OpenFileProps & { onRestored: () => void }) {
  const [revisions, setRevisions] = React.useState<ProjectFileRevisionView[] | null>(null);
  const [loading, setLoading] = React.useState(false);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [selected, setSelected] = React.useState<ProjectFileRevisionView | null>(null);
  const [restoring, setRestoring] = React.useState(false);
  const [restoreError, setRestoreError] = React.useState<string | null>(null);
  const [expandedId, setExpandedId] = React.useState<string | null>(null);
  const changesIdPrefix = React.useId();
  const loadRequestRef = React.useRef(0);
  const restoreRequestRef = React.useRef(0);

  // A reload does not clear `revisions` up front: the list already on
  // screen is real, read history, not a draft to discard on a hunch. A
  // failed reload states the failure without also taking away the answer
  // the previous load already gave.
  const load = React.useCallback(() => {
    const request = ++loadRequestRef.current;
    setLoading(true);
    setLoadError(null);
    projectFileRevisions(projectId, path).then(
      (result) => {
        if (request !== loadRequestRef.current) return;
        setLoading(false);
        if (!Array.isArray(result.revisions)) {
          setLoadError("Revision history returned invalid data. Try again.");
          return;
        }
        const uniqueIds = new Set(result.revisions.map((revision) => revision.backup_id));
        if (
          result.project_id !== projectId ||
          result.relative_path !== path ||
          uniqueIds.size !== result.revisions.length ||
          result.revisions.some(
            (revision) =>
              revision.version !== 1 ||
              revision.backup_id.trim() === "" ||
              !Number.isFinite(revision.created_at_ms) ||
              !Number.isFinite(revision.original_bytes) ||
              revision.original_bytes < 0 ||
              !Number.isFinite(revision.compressed_bytes) ||
              revision.compressed_bytes < 0 ||
              revision.content_hash.trim() === "",
          )
        ) {
          setLoadError("Revision history returned invalid data. Try again.");
          return;
        }
        setRevisions(result.revisions);
      },
      (cause: unknown) => {
        if (request === loadRequestRef.current) {
          setLoading(false);
          setLoadError(describeProjectError(cause, "Revision history could not be loaded."));
        }
      },
    );
  }, [path, projectId]);

  React.useEffect(() => {
    load();
    return () => {
      loadRequestRef.current += 1;
      restoreRequestRef.current += 1;
    };
  }, [load]);

  const restore = async () => {
    if (selected == null || restoring) return;
    const request = ++restoreRequestRef.current;
    setRestoring(true);
    setRestoreError(null);
    try {
      const result = await projectFileRestore(selected.backup_id);
      if (request !== restoreRequestRef.current) return;
      if (result.project_id !== projectId || result.relative_path !== path) {
        setRestoreError("The restored revision did not match this file. Reload history and try again.");
        setRestoring(false);
        return;
      }
      setRestoring(false);
      setSelected(null);
      onRestored();
    } catch (cause: unknown) {
      if (request === restoreRequestRef.current) {
        setRestoreError(describeProjectError(cause, "This revision could not be restored."));
        setRestoring(false);
      }
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 p-4">
      <div className="flex items-center justify-between gap-3">
        {/* The tab already names this panel; the caption says what it holds. */}
        <p className="type-caption text-muted">Stored on this device before xFalcon replaced this file.</p>
        <Button disabled={loading} onClick={load} size="sm" variant="secondary">
          {loading ? <Spinner decorative size={14} /> : <RotateCcw aria-hidden size={14} />}
          Reload history
        </Button>
      </div>
      {loadError ? (
        <Alert
          action={
            <Button disabled={loading} onClick={load} size="sm" variant="secondary">
              Try again
            </Button>
          }
          title="Revision history was not loaded"
          tone="danger"
        >
          {loadError}
        </Alert>
      ) : null}
      {revisions == null ? (
        loading ? (
          <p className="type-body flex items-center gap-2 text-secondary">
            <Spinner decorative size={16} /> Loading revisions…
          </p>
        ) : null
      ) : revisions.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-default p-8 text-center">
          <History aria-hidden className="text-muted" size={22} />
          <p className="type-body font-medium text-foreground">No revisions yet</p>
          <p className="type-caption max-w-sm text-muted">
            A revision appears here before xFalcon next replaces this file.
          </p>
        </div>
      ) : (
        <ul className="flex flex-col divide-y divide-hairline overflow-auto surface-card">
          {revisions.map((revision, index) => {
            const expanded = expandedId === revision.backup_id;
            const changesId = `${changesIdPrefix}-${revision.backup_id}`;
            return (
              <li className="flex flex-wrap items-center gap-3 px-3 py-2.5" key={revision.backup_id}>
                <div className="min-w-0 flex-1">
                  <p className="type-body font-medium text-foreground">{formatRevisionDate(revision.created_at_ms)}</p>
                  <p className="type-caption text-muted">
                    {formatBytes(revision.original_bytes)} · {revision.content_hash.slice(0, 12)}
                  </p>
                </div>
                <Button
                  aria-controls={expanded ? changesId : undefined}
                  aria-expanded={expanded}
                  onClick={() => setExpandedId(expanded ? null : revision.backup_id)}
                  size="sm"
                  variant="ghost"
                >
                  {expanded ? "Hide changes" : "Show changes"}
                </Button>
                <Button
                  onClick={() => {
                    setRestoreError(null);
                    setSelected(revision);
                  }}
                  size="sm"
                  variant="secondary"
                >
                  Restore revision
                </Button>
                {expanded ? (
                  <RevisionChanges
                    backupId={revision.backup_id}
                    id={changesId}
                    // Newest first, so the revision after this one is the entry above it.
                    next={index > 0 ? revisions[index - 1] : null}
                    path={path}
                    projectId={projectId}
                  />
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
      <AlertDialog
        busy={restoring}
        confirmLabel="Restore this revision"
        description={
          selected == null
            ? ""
            : `This replaces ${path} with the ${formatRevisionDate(selected.created_at_ms)} revision. xFalcon saves the current file first, so you can restore it again later.`
        }
        error={restoreError}
        onCancel={() => {
          if (restoring) return;
          restoreRequestRef.current += 1;
          setRestoreError(null);
          setSelected(null);
        }}
        onConfirm={() => void restore()}
        open={selected != null}
        title="Restore this file?"
      />
    </div>
  );
}

function formatRevisionDate(createdAtMs: number): string {
  if (!Number.isFinite(createdAtMs)) return "unknown date";
  const date = new Date(createdAtMs);
  if (Number.isNaN(date.getTime())) return "unknown date";
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(date);
}

/*
 * Memoized because its owner re-renders on every filter keystroke, folder
 * toggle and listing refresh, while the file on screen stays the same. Each
 * such render would otherwise re-run the whole file view, including the full
 * table analysis of an open CSV.
 */
export const ProjectFileViewer = React.memo(function ProjectFileViewer({
  projectId,
  path,
  drafts,
}: ProjectFileViewerProps) {
  const [tab, setTab] = React.useState("file");
  const [fileVersion, setFileVersion] = React.useState(0);

  React.useEffect(() => setTab("file"), [projectId, path]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <Tabs
        aria-label="File sections"
        className="mx-4 mt-3 self-start"
        onChange={setTab}
        panelClassName="flex min-h-0 flex-1 flex-col"
        tabs={[
          { id: "file", label: "File" },
          { id: "revisions", label: "History & Revisions", disabled: path === null },
          { id: "schemas", label: "Schemas" },
        ]}
        value={tab}
      >
        {tab === "schemas" ? (
          <ProjectSchemaViewer projectId={projectId} path={path} />
        ) : path === null ? (
          <div className="flex min-h-0 flex-1 items-center justify-center p-6">
            <EmptyState
              description="Pick a file in the tree to read it. Files xFalcon classifies as sensitive are listed but never opened."
              title={<p className="type-body font-semibold text-foreground">No file selected</p>}
            />
          </div>
        ) : tab === "file" ? (
          isPreviewableOfficePath(path) ? (
            <ProjectOfficeViewer key={`${projectId}:${path}:${fileVersion}`} path={path} projectId={projectId} />
          ) : (
            <ProjectFileContents
              drafts={drafts}
              key={`${projectId}:${path}:${fileVersion}`}
              path={path}
              projectId={projectId}
            />
          )
        ) : (
          <RevisionsPanel
            onRestored={() => {
              setFileVersion((version) => version + 1);
              setTab("file");
            }}
            path={path}
            projectId={projectId}
          />
        )}
      </Tabs>
    </div>
  );
});
