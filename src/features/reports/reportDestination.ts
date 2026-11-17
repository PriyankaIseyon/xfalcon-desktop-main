import { createContext, useContext } from "react";

/**
 * Where "Save to project" saves for the open conversation (spec: chat-rich-content,
 * "The save offer names its destination").
 *
 * A conversation bound to a project saves its dashboards into that project's
 * folder. A conversation with no project offers no save control. The shell knows
 * the open conversation's binding and provides it here. A context rather than a
 * prop for the reason `reportsStore`'s open intent gives: the alternative is a
 * prop threaded through `ChatSurface`, `MessageList`, `MessageRow` and
 * `AssistantMessage`, none of which has any interest in reports.
 *
 * `conversationId` travels with the project so a card can check the destination
 * is its own conversation's. The context describes the OPEN conversation, and a
 * card that rendered for another one must not borrow its folder.
 */
export type ProjectReportDestination = {
  conversationId: string;
  projectId: string;
  projectName: string;
  /** Opens a project-relative file in this project's file view. */
  openFile: (relativePath: string) => void;
};

export const ReportDestinationContext = createContext<ProjectReportDestination | null>(null);

/** The project a card for `conversationId` saves into, or `null` when it has none. */
export function useProjectReportDestination(conversationId: string): ProjectReportDestination | null {
  const destination = useContext(ReportDestinationContext);
  return destination !== null && destination.conversationId === conversationId ? destination : null;
}

/**
 * The projects a report can be copied into from the Reports library, and how to
 * open a file in one of them afterwards.
 *
 * Distinct from [`ProjectReportDestination`], which is the ONE project the open
 * conversation is bound to: the library is not in a conversation, so the reader
 * chooses. Only a trusted project whose folder is reachable is offered. `null`
 * (no provider, as in a bare test) offers no copy control at all.
 */
export type ReportProjectChoices = {
  projects: { id: string; name: string }[];
  /** The project the reader is working in, offered first. */
  activeProjectId: string | null;
  /** Opens a project-relative file in that project's file view. */
  openFile: (projectId: string, relativePath: string) => void;
};

export const ReportProjectsContext = createContext<ReportProjectChoices | null>(null);
