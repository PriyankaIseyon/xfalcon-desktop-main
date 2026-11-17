import { ChevronDown, LogOut } from "lucide-react";
import type { AppBootstrap, IdentityProjection } from "../lib/generated/contracts";
import { Avatar } from "../ui/Avatar";
import { Menu } from "../ui/Menu";
import { XFalconMark } from "../ui/XFalconMark";

function platformName(platform: AppBootstrap["platform"]): string {
  const names: Record<AppBootstrap["platform"], string> = {
    windows: "Windows",
    macos: "macOS",
    linux: "Linux",
    unknown: "Desktop",
  };
  return names[platform];
}

/**
 * Who is signed in, and the session actions that belong to them.
 *
 * One component for both sidebar densities. The open sidebar shows the name and
 * email on a card; a rail has room for the avatar alone. The *menu* is the
 * same either way -- sign-out is the only way out of the app, and a person who
 * has collapsed the sidebar has not asked to lose it.
 */
export interface AccountMenuProps {
  identity: IdentityProjection | null;
  platform: AppBootstrap["platform"];
  applicationVersion: string;
  /** `card` for the open sidebar, `compact` for the rail. */
  variant?: "card" | "compact";
  onSignOut: () => void;
}

export function AccountMenu({ identity, platform, applicationVersion, variant = "card", onSignOut }: AccountMenuProps) {
  const name = identity?.display_name || "Account";
  const detail = identity ? identity.email : `${platformName(platform)} · v${applicationVersion}`;
  const compact = variant === "compact";
  const avatar = (
    <Avatar aria-hidden className="app-account-menu__avatar size-6 rounded-full" size="sm">
      <XFalconMark />
    </Avatar>
  );

  // A literal button, not a wrapper component: `Menu` clones its trigger to
  // inject the click handler and the ref it focuses on close, and neither
  // survives a component that does not forward them.
  const trigger = compact ? (
    <button aria-label="Account and app menu" className="app-sidebar__rail-item touch-target" type="button">
      {avatar}
    </button>
  ) : (
    <button aria-label="Account and app menu" className="app-account-trigger app-account-trigger-card" type="button">
      <div className="flex min-w-0 flex-1 items-center gap-2">
        {avatar}
        <span className="flex min-w-0 flex-1 flex-col text-left">
          <strong className="text-xs font-semibold text-foreground truncate">{name}</strong>
          {detail ? <small className="type-micro text-muted truncate">{detail}</small> : null}
        </span>
      </div>
      <ChevronDown aria-hidden size={13} className="ml-auto shrink-0 text-muted" />
    </button>
  );

  return (
    <div className="app-account-menu">
      <Menu
        // A rail is 3.5rem wide, so a popover pinned to the trigger's right
        // edge would open off the left of the window.
        align={compact ? "start" : "end"}
        className="app-account-popover"
        side="top"
        trigger={trigger}
        triggerTooltip={compact ? { label: `Account · ${name}`, side: "right" } : undefined}
      >
        <Menu.Label>Session</Menu.Label>
        <Menu.Item className="app-account-popover__item" icon={<LogOut aria-hidden size={14} />} onSelect={onSignOut}>
          Sign out
        </Menu.Item>
      </Menu>
    </div>
  );
}
