import { ChartNoAxesColumn, Database, Folders, MessageSquare, Settings, type LucideIcon } from "lucide-react";

/** The one workspace surface the main region is showing. Chat is the default. */
export type WorkspaceView = "chat" | "projects" | "datasets" | "reports" | "settings";

export interface Destination {
  view: WorkspaceView;
  label: string;
  icon: LucideIcon;
}

/**
 * The destinations the sidebar offers as one group, in one order.
 *
 * Stated once because the sidebar draws them twice -- as labelled rows when it
 * is open, as icons when it is a rail -- and two hand-written lists of the same
 * destinations is how a rail ends up in a different order, or missing the one
 * that shipped last.
 *
 * Chats and Projects come first because they are the two surfaces you work in,
 * and each owns the list below the group while it is current. Datasets and
 * Reports are what you consult, and bring no list of their own.
 */
export const PRIMARY_DESTINATIONS: readonly Destination[] = [
  { view: "chat", label: "Chats", icon: MessageSquare },
  { view: "projects", label: "Projects", icon: Folders },
  { view: "datasets", label: "Datasets", icon: Database },
  { view: "reports", label: "Reports", icon: ChartNoAxesColumn },
];

/**
 * Settings is a destination, but not one you work in: it sits with the account
 * at the foot of the sidebar, where an app's own settings belong.
 */
export const SETTINGS_DESTINATION: Destination = { view: "settings", label: "Settings", icon: Settings };
