import { useSyncExternalStore } from "react";
import { Download, RotateCw } from "lucide-react";
import { Button } from "../../ui/Button";
import { usePlatform } from "../../lib/platform";
import { Progress } from "../../ui/Progress";
import { checkForUpdates, dismissUpdateNotice, getUpdateSnapshot, restartApp, subscribeUpdate } from "./updateStore";

/** Remains reachable on every workspace, without interrupting the user's work. */
export function UpdateNotice({ centered = false }: { centered?: boolean }) {
  const platform = usePlatform();
  const update = useSyncExternalStore(subscribeUpdate, getUpdateSnapshot, getUpdateSnapshot);
  if (update.noticeDismissed || !["installing", "ready_to_restart", "error"].includes(update.status)) {
    return null;
  }
  const downloading = update.status === "installing";
  const ready = update.status === "ready_to_restart";
  const failed = update.status === "error";
  return (
    <section aria-label="Software update" className="shrink-0 border-b border-default bg-card px-4 py-3">
      <div
        className={centered ? "flex flex-wrap items-center justify-center gap-3" : "flex flex-wrap items-center gap-3"}
      >
        {ready ? <RotateCw aria-hidden size={16} /> : <Download aria-hidden size={16} />}
        <div className={centered ? "min-w-0 text-center" : "min-w-0 flex-1"} role="status" aria-live="polite">
          <p className="type-caption font-medium text-foreground">
            {failed
              ? "Update could not finish"
              : ready
                ? `Version ${update.availableVersion} is ready to install`
                : `Downloading version ${update.availableVersion}`}
          </p>
          <p className="type-caption text-secondary">
            {update.errorMessage ??
              (ready
                ? "Restart when you are ready. The update has been verified."
                : downloading
                  ? "Keep working. You choose when to restart."
                  : `Installed version: ${update.currentVersion}`)}
          </p>
          {ready && platform === "macos" && (
            <p className="type-caption text-secondary">
              Updates require a writable Applications folder. macOS may ask for an administrator password. If macOS
              blocks replacement, install the downloaded release manually.
            </p>
          )}
        </div>
        {!downloading && (
          <Button size="sm" variant="ghost" disabled={update.restarting} onClick={dismissUpdateNotice}>
            Later
          </Button>
        )}
        {ready && update.errorMessage && (
          <Button size="sm" disabled={update.restarting} onClick={() => void checkForUpdates()}>
            Check again
          </Button>
        )}
        {ready ? (
          <Button size="sm" variant="primary" loading={update.restarting} onClick={() => void restartApp()}>
            Restart to update
          </Button>
        ) : failed ? (
          <Button size="sm" onClick={() => void checkForUpdates()}>
            Try again
          </Button>
        ) : null}
      </div>
      {downloading && (
        <div className={centered ? "mx-auto mt-2 w-full max-w-md" : "mt-2"}>
          <Progress
            label="Update download progress"
            indeterminate={update.downloadProgress === null}
            value={update.downloadProgress ?? 0}
          />
        </div>
      )}
    </section>
  );
}
