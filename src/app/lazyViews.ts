import { lazy } from "react";

/*
 * Views the first screen never shows. Each one is its own chunk, fetched when
 * it first renders, so the launch parse does not pay for them. The loaders are
 * named so `preloadWorkspaceViews` can start the same fetch ahead of time.
 */
const loadSettingsView = () =>
  import("../features/settings/SettingsView").then((module) => ({ default: module.SettingsView }));
const loadProjectExplorerView = () =>
  import("../features/projects/ProjectExplorerView").then((module) => ({ default: module.ProjectExplorerView }));
const loadDatasetsView = () =>
  import("../features/datasets/DatasetsView").then((module) => ({ default: module.DatasetsView }));
const loadReportsView = () =>
  import("../features/reports/ReportsView").then((module) => ({ default: module.ReportsView }));
const loadCanvasPanel = () =>
  import("../features/canvas/CanvasPanel").then((module) => ({ default: module.CanvasPanel }));
const loadChatExportDialog = () =>
  import("../features/chat/ChatExportDialog").then((module) => ({ default: module.ChatExportDialog }));

export const SettingsView = lazy(loadSettingsView);
export const ProjectExplorerView = lazy(loadProjectExplorerView);
export const DatasetsView = lazy(loadDatasetsView);
export const ReportsView = lazy(loadReportsView);
export const CanvasPanel = lazy(loadCanvasPanel);
export const ChatExportDialog = lazy(loadChatExportDialog);

const workspaceViewLoaders = [
  loadSettingsView,
  loadProjectExplorerView,
  loadDatasetsView,
  loadReportsView,
  loadCanvasPanel,
  loadChatExportDialog,
];

/**
 * Starts fetching every lazy view, so the first visit does not wait on the
 * network. Failures are dropped here on purpose: React.lazy runs the same
 * import again when the view renders, and that render throws into
 * AppErrorBoundary, so a broken chunk still surfaces where the person meets it.
 */
export function preloadWorkspaceViews(): void {
  for (const load of workspaceViewLoaders) {
    void load().catch(() => undefined);
  }
}
