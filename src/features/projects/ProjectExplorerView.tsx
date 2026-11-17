import React from "react";
import { ChevronsDownUp, ChevronsUpDown, FolderPlus, FolderTree, RotateCcw } from "lucide-react";
import type { ProjectView } from "../../lib/generated/contracts";
import { displayPath } from "../../lib/displayPath";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { EmptyState } from "../../ui/EmptyState";
import { IconButton } from "../../ui/IconButton";
import { SearchField } from "../../ui/SearchField";
import { Spinner } from "../../ui/Spinner";
import { TruncatedText } from "../../ui/TruncatedText";
import { ProjectFileViewer, type UnsavedDraftStore } from "./ProjectFileViewer";
import { ProjectCreateSheet } from "./ProjectCreateSheet";
import { ProjectTree } from "./ProjectTree";
import { useProjectTree } from "./useProjectTree";
import { filterProjectRows } from "./treeFilter";
import { HighlightedText, SearchRevealHost, SearchRevealProvider } from "../../lib/highlightedText";

export interface ProjectExplorerViewProps {
  /** The active trusted project, or `null` when none is open. */
  project: ProjectView | null;
  /**
   * The project-relative path this view has open, or `null` for none. Owned by
   * the shell rather than here: the sidebar opens files too, and two owners of
   * "which file is open" would disagree the moment either one moved.
   */
  selectedPath: string | null;
  revealQuery?: string;
  onSelectPath: (path: string) => void;
  onProjectReady: (project: ProjectView) => void;
}

export function ProjectExplorerView({
  project,
  selectedPath,
  onSelectPath,
  onProjectReady,
  revealQuery = "",
}: ProjectExplorerViewProps) {
  const [createOpen, setCreateOpen] = React.useState(false);
  if (project == null) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center p-6">
        <EmptyState
          align="center"
          description="Add a local folder as a project, or select an existing project from the sidebar."
          icon={<FolderTree aria-hidden size={28} strokeWidth={1.6} />}
          title={<p className="type-body font-semibold text-foreground">No project is open</p>}
          actions={
            <Button onClick={() => setCreateOpen(true)} variant="primary">
              <FolderPlus aria-hidden size={16} strokeWidth={1.8} />
              New project
            </Button>
          }
        />
        <ProjectCreateSheet
          open={createOpen}
          onClose={() => setCreateOpen(false)}
          onReady={(created) => {
            setCreateOpen(false);
            onProjectReady(created);
          }}
        />
      </div>
    );
  }

  // Local controls belong to one project. Keying the content resets them in
  // the same render as a project switch, before an old open schema panel can
  // issue a request for the new project or an old filter can hide its files.
  return (
    <OpenProjectExplorer
      key={project.id}
      onSelectPath={onSelectPath}
      project={project}
      revealQuery={revealQuery}
      selectedPath={selectedPath}
    />
  );
}

function OpenProjectExplorer({
  project,
  selectedPath,
  onSelectPath,
  revealQuery,
}: Omit<ProjectExplorerViewProps, "project" | "onProjectReady"> & { project: ProjectView }) {
  // An unavailable root must not start a filesystem request. It remains in the
  // project list so its conversations stay readable, but browsing fails closed.
  const tree = useProjectTree(project.status === "trusted" ? project.id : null);
  const [searchQuery, setSearchQuery] = React.useState("");
  // Unsaved edits, kept across opening other files and tabs for as long as this
  // project stays open. See `UnsavedDraftStore`.
  const drafts = React.useRef<UnsavedDraftStore>(new Map()).current;

  const rootError = tree.errors.get("");
  const normalizedQuery = searchQuery.trim().toLocaleLowerCase();
  const filteredRows = React.useMemo(() => filterProjectRows(tree.rows, normalizedQuery), [normalizedQuery, tree.rows]);

  return (
    <SearchRevealProvider value={revealQuery ? { query: revealQuery, targetId: "project-reveal" } : null}>
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-hairline bg-surface/30 px-5 py-3.5 backdrop-blur-xs">
          <SearchRevealHost id="project-reveal">
            <div className="flex min-w-0 max-w-full items-center gap-2.5">
              {project.status === "trusted" && tree.rootStatus === "loading" ? <Spinner decorative size={14} /> : null}
              <TruncatedText
                className="type-caption min-w-0 rounded-md border border-default bg-card/80 px-3 py-1 font-mono text-muted shadow-e1"
                text={displayPath(project.root_path)}
              >
                <HighlightedText text={displayPath(project.root_path)} />
              </TruncatedText>
              {project.status === "trusted" && tree.rootStatus === "ready" ? (
                <span className="type-caption shrink-0 text-muted" role="status">
                  {tree.rows.length === 1 ? "1 visible item" : `${tree.rows.length} visible items`}
                </span>
              ) : null}
            </div>
          </SearchRevealHost>
        </div>

        {project.status === "unavailable" ? (
          <div className="flex min-h-0 flex-1 items-center justify-center p-6">
            <Alert className="w-full max-w-2xl" title="This project folder is not available" tone="warn">
              The trusted folder cannot be reached. Reconnect its drive or restore the folder at its original location,
              then reload the project list.
            </Alert>
          </div>
        ) : (
          <div className="grid min-h-0 flex-1 grid-cols-1 grid-rows-[minmax(12rem,40vh)_minmax(16rem,1fr)] md:grid-cols-[minmax(14rem,20rem)_minmax(0,1fr)] md:grid-rows-1">
            <div className="flex min-h-0 flex-col border-hairline md:border-r bg-surface/10">
              <div className="flex items-center gap-1.5 border-b border-hairline px-3 py-2">
                <SearchField
                  aria-label="Filter visible project files"
                  className="min-w-0 flex-1"
                  onChange={(event) => setSearchQuery(event.currentTarget.value)}
                  onClear={() => setSearchQuery("")}
                  placeholder="Filter visible files…"
                  value={searchQuery}
                />
                {/* One control that turns into its opposite, as an IDE's
                    explorer does: collapse while anything is open, expand
                    once nothing is. */}
                <IconButton
                  aria-label={tree.anyExpanded ? "Collapse all folders" : "Expand all folders"}
                  disabled={tree.rootStatus !== "ready"}
                  onClick={tree.anyExpanded ? tree.collapseAll : tree.expandAll}
                  size="md"
                >
                  {tree.anyExpanded ? (
                    <ChevronsDownUp aria-hidden size={15} />
                  ) : (
                    <ChevronsUpDown aria-hidden size={15} />
                  )}
                </IconButton>
                <IconButton aria-label="Reload tree" onClick={tree.reload} size="md">
                  <RotateCcw aria-hidden size={15} />
                </IconButton>
              </div>

              {tree.rootStatus === "loading" ? (
                <p className="type-body flex items-center gap-2 px-3 py-3 text-secondary">
                  <Spinner decorative size={16} />
                  Listing files…
                </p>
              ) : null}

              {rootError ? (
                <div className="p-3">
                  <Alert title="This project could not be listed" tone="danger">
                    <p>{rootError}</p>
                    <Button className="mt-2" onClick={tree.reload} size="sm" variant="secondary">
                      Try again
                    </Button>
                  </Alert>
                </div>
              ) : null}

              {tree.rootStatus === "ready" && filteredRows.length === 0 ? (
                <p className="type-body px-3 py-3 text-secondary">
                  {normalizedQuery ? "No visible files match this filter." : "This project folder is empty."}
                </p>
              ) : null}

              {filteredRows.length > 0 ? (
                <ProjectTree
                  errors={tree.errors}
                  onOpenFile={(row) => onSelectPath(row.path)}
                  onToggle={tree.toggle}
                  rows={filteredRows}
                  selectedPath={selectedPath}
                />
              ) : null}
            </div>

            <div className="flex min-h-0 flex-col bg-card/40">
              {selectedPath == null ? null : (
                <div className="flex items-center justify-between border-b border-hairline bg-surface/20 px-4 py-2">
                  <TruncatedText
                    className="type-caption min-w-0 rounded-md bg-card px-2.5 py-0.5 font-mono text-secondary shadow-e1"
                    text={selectedPath}
                  >
                    {selectedPath}
                  </TruncatedText>
                </div>
              )}
              <ProjectFileViewer
                drafts={drafts}
                key={`${project.id}:${selectedPath ?? ""}`}
                path={selectedPath ?? null}
                projectId={project.id}
              />
            </div>
          </div>
        )}
      </div>
    </SearchRevealProvider>
  );
}
