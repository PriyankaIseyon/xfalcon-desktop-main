import { useSyncExternalStore } from "react";
import { IconButton } from "../../ui/IconButton";
import { getUpdateSnapshot, subscribeUpdate } from "./updateStore";

export function SettingsUpdateBadge({ onOpenUpdates }: { onOpenUpdates: () => void }) {
  const update = useSyncExternalStore(subscribeUpdate, getUpdateSnapshot, getUpdateSnapshot);
  const count = update.availableVersion && !["idle", "up_to_date"].includes(update.status) ? 1 : 0;
  return (
    <>
      <span id="settings-update-count" className="sr-only" role="status">
        {count ? `${count} update available` : "No updates"}
      </span>
      {count > 0 && (
        <IconButton
          aria-label="New update available. Open software updates"
          className="absolute right-1 top-1/2 -translate-y-1/2"
          shape="circle"
          tooltipSide="right"
          onClick={onOpenUpdates}
        >
          <span
            aria-hidden="true"
            className="inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-accent type-caption font-semibold tabular-nums text-accent-ink"
          >
            {count}
          </span>
        </IconButton>
      )}
    </>
  );
}
