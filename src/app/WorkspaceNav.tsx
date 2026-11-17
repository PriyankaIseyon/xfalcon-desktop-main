import type { ReactNode } from "react";
import { MessageSquarePlus, Plus } from "lucide-react";
import { IconButton } from "../ui/IconButton";
import { Logo } from "../ui/Logo";
import { Tooltip } from "../ui/Tooltip";
import { PRIMARY_DESTINATIONS, SETTINGS_DESTINATION, type WorkspaceView } from "./destinations";

/**
 * The destinations, as one group at the top of the sidebar.
 *
 * They sit together in a well rather than scattered down the column because
 * they answer one question -- which surface the main region shows -- and a
 * reader should be able to answer it without reading the whole sidebar. The
 * current one is raised out of the well and marked with the accent, the way
 * every other "this is the one" in the app is. Whichever one is current also
 * owns the column below the group, so the list under the well always belongs
 * to the destination the well says you are on.
 *
 * `aria-current` carries the state, and the stylesheet reads it: a second
 * `active` class saying the same thing is a second thing to keep in sync.
 */
export interface DestinationNavProps {
  view: WorkspaceView;
  onNavigate: (view: WorkspaceView) => void;
  /**
   * Starts a new item of that destination's kind from any page: a chat, a
   * project, or a dataset upload. Shown as a "+" on the row, revealed on hover
   * and on keyboard focus, and always visible on a touch screen (hover is never
   * the only path).
   */
  onCreate?: (view: WorkspaceView) => void;
  /** Whether a chat can start now. The Chats "+" is disabled until it can, as the sidebar's New chat button is. */
  chatReady: boolean;
}

/**
 * The destinations that can start a new item, and what the "+" says. Worded
 * apart from the sidebar's own "New chat" and "New project" buttons, so a
 * screen reader never hears two controls with one name.
 */
const CREATE_LABEL: Partial<Record<WorkspaceView, string>> = {
  chat: "Start a new chat",
  projects: "Create a project",
  datasets: "Upload a dataset",
};

export function DestinationNav({ view, chatReady, onNavigate, onCreate }: DestinationNavProps) {
  const activeIndex = PRIMARY_DESTINATIONS.findIndex(({ view: target }) => view === target);
  return (
    <div className="app-sidebar__destinations" data-active-index={activeIndex}>
      <span aria-hidden className="nav-destination__highlight" />
      {PRIMARY_DESTINATIONS.map(({ view: target, label, icon: Icon }) => {
        const active = view === target;
        const createLabel = onCreate ? CREATE_LABEL[target] : undefined;
        return (
          <div key={target} className="nav-destination-row">
            <button
              aria-current={active ? "page" : undefined}
              className="nav-destination"
              type="button"
              onClick={() => onNavigate(target)}
            >
              {/* The one mark that means "current" in this group. It is absolutely
                placed so a destination's label sits at the same inset whether or
                not it is the one you are on. */}
              {active ? <span aria-hidden className="nav-destination__marker" /> : null}
              {/* An icon beside text carries that text's optical weight, and the
                current destination's label is the one that steps up to 600. */}
              <Icon aria-hidden size={16} strokeWidth={active ? 2.2 : 1.8} />
              <span>{label}</span>
            </button>
            {createLabel ? (
              <IconButton
                aria-label={createLabel}
                className="nav-destination__create"
                disabled={target === "chat" && !chatReady}
                size="sm"
                onClick={() => onCreate?.(target)}
              >
                <Plus aria-hidden size={14} strokeWidth={1.8} />
              </IconButton>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}

/**
 * The sidebar, collapsed to icons.
 *
 * Collapsing used to take the sidebar to nothing, which meant giving up the
 * workspace's whole navigation to get the width back. A rail keeps every
 * destination one press away at a twelfth of the width, and keeps the current
 * one visible -- so the reader never loses their place to gain room.
 *
 * Each icon uses the product tooltip on hover and keyboard focus. The label is
 * not decoration. It gives an unfamiliar icon a visible meaning.
 */
export interface SidebarRailProps {
  view: WorkspaceView;
  onNavigate: (view: WorkspaceView) => void;
  onNewChat: () => void;
  newChatDisabled?: boolean;
  /** Rendered as-is at the foot of the rail. Owned by the shell, because the
   * rail must not know how a session ends. */
  account: ReactNode;
  /** Makes the rail a window drag handle. Only on macOS, where the drag permissions are granted. */
  windowDrag?: boolean;
}

export function SidebarRail({
  view,
  onNavigate,
  onNewChat,
  newChatDisabled = false,
  account,
  windowDrag = false,
}: SidebarRailProps) {
  const SettingsIcon = SETTINGS_DESTINATION.icon;
  return (
    <nav aria-label="Workspace" className="app-sidebar__rail" data-tauri-drag-region={windowDrag ? true : undefined}>
      {/* The mark holds the same top-left position it holds when the sidebar is
          open, so collapsing does not move the app's own identity. */}
      <Logo className="app-sidebar__rail-mark" showWordmark={false} />
      <span aria-hidden className="app-sidebar__rail-divider" />

      {PRIMARY_DESTINATIONS.map(({ view: target, label, icon: Icon }) => (
        <RailItem key={target} current={view === target} label={label} onClick={() => onNavigate(target)}>
          <Icon aria-hidden size={18} strokeWidth={view === target ? 2.2 : 1.8} />
        </RailItem>
      ))}

      {/* Starting a chat is the one action worth a rail slot, and only while
          chat is the surface you are on. */}
      {view === "chat" ? (
        <RailItem disabled={newChatDisabled} label="New chat" onClick={onNewChat}>
          <MessageSquarePlus aria-hidden size={18} strokeWidth={1.8} className="text-accent" />
        </RailItem>
      ) : null}

      <span aria-hidden className="app-sidebar__rail-spacer" />

      <RailItem
        current={view === SETTINGS_DESTINATION.view}
        label={SETTINGS_DESTINATION.label}
        onClick={() => onNavigate(SETTINGS_DESTINATION.view)}
      >
        <SettingsIcon aria-hidden size={18} strokeWidth={1.8} />
      </RailItem>
      {account}
    </nav>
  );
}

function RailItem({
  children,
  current = false,
  disabled = false,
  label,
  onClick,
}: {
  children: ReactNode;
  current?: boolean;
  disabled?: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <Tooltip label={label} side="right">
      <button
        aria-current={current ? "page" : undefined}
        aria-label={label}
        className="app-sidebar__rail-item touch-target"
        disabled={disabled}
        type="button"
        onClick={onClick}
      >
        {children}
      </button>
    </Tooltip>
  );
}
