import React, { useSyncExternalStore } from "react";
import { usePlatform } from "../../lib/platform";
import { Spinner } from "../../ui/Spinner";
import { Progress } from "../../ui/Progress";
import { checkForUpdates, getUpdateSnapshot, initAppVersion, restartApp, subscribeUpdate } from "./updateStore";
import { SettingsButton } from "./SettingsControls";
import { SettingsActions, SettingsGroup, SettingsRow } from "./SettingsLayout";
import { useTaps } from "../delight/useTaps";
import { ReleaseNotesMarkdown } from "./ReleaseNotesMarkdown";
import { VersionThanks } from "./VersionThanks";
import "./UpdateSection.css";

const SIGNATURE_TAPS = 7;

function formatCheckTime(timestamp: number | null): string {
  if (!timestamp) return "Never";
  const secondsAgo = Math.floor((Date.now() - timestamp) / 1000);
  if (secondsAgo < 60) return "Just now";
  const minutesAgo = Math.floor(secondsAgo / 60);
  if (minutesAgo < 60) return `${minutesAgo} minute${minutesAgo === 1 ? "" : "s"} ago`;
  const hoursAgo = Math.floor(minutesAgo / 60);
  return `${hoursAgo} hour${hoursAgo === 1 ? "" : "s"} ago`;
}

function formatCheckHint(timestamp: number | null, succeeded: boolean | null): string {
  const outcome = succeeded === null ? "" : succeeded ? " · Succeeded" : " · Failed";
  return `Last checked: ${formatCheckTime(timestamp)}${outcome}`;
}

/** Software updates, as a block inside ApplicationSection (no box of its own). */
export function UpdateSection() {
  const platform = usePlatform();
  const update = useSyncExternalStore(subscribeUpdate, getUpdateSnapshot, getUpdateSnapshot);
  const [showThanks, setShowThanks] = React.useState(false);

  React.useEffect(() => {
    if (update.currentVersion === "…") {
      void initAppVersion();
    }
  }, [update.currentVersion]);

  // Nothing about the version looks or feels clickable, on purpose: no role,
  // cursor or hover, so nothing invites the click.
  const signOff = useTaps(SIGNATURE_TAPS, () => setShowThanks(true));

  const isChecking = update.status === "checking";
  const isInstalling = update.status === "installing";
  const isReadyToRestart = update.status === "ready_to_restart";

  return (
    <SettingsGroup id="settings-updates" aria-label="Software updates">
      <SettingsRow
        className="settings-update-row"
        label={
          <>
            xFalcon Desktop{" "}
            <span className="text-secondary" onClick={signOff}>
              v{update.currentVersion}
            </span>
          </>
        }
        hint={<>{formatCheckHint(update.lastCheckTime, update.lastCheckSucceeded)}</>}
      >
        <div className="settings-update-feedback">
          {update.status === "up_to_date" && (
            <span className="settings-update-feedback__badge type-label" role="status">
              xFalcon is up to date.
            </span>
          )}
        </div>
        <SettingsActions>
          {isInstalling && (
            <SettingsButton disabled loading variant="primary">
              {update.downloadProgress === null ? "Downloading…" : `Downloading (${update.downloadProgress}%)`}
            </SettingsButton>
          )}

          {isReadyToRestart && (
            <>
              {update.errorMessage && (
                <SettingsButton disabled={update.restarting} onClick={() => void checkForUpdates()}>
                  Check again
                </SettingsButton>
              )}
              <SettingsButton variant="primary" loading={update.restarting} onClick={() => void restartApp()}>
                Restart app
              </SettingsButton>
            </>
          )}

          {!isInstalling && !isReadyToRestart && (
            <SettingsButton disabled={isChecking} onClick={() => void checkForUpdates(false)}>
              {isChecking ? (
                <span className="flex items-center gap-1.5">
                  <Spinner decorative size={12} />
                  Checking…
                </span>
              ) : (
                "Check for updates"
              )}
            </SettingsButton>
          )}
        </SettingsActions>
      </SettingsRow>

      {update.status === "ready_to_restart" && !update.errorMessage && (
        <p className="settings-group__state text-success">
          Update downloaded and verified. Restart xFalcon to install.
        </p>
      )}

      {(isInstalling || isReadyToRestart) && update.releaseNotes && (
        <div className="settings-group__state flex-col items-start">
          <p className="type-caption font-medium text-foreground">Version {update.availableVersion} release notes</p>
          <div className="mt-1 w-full min-w-0 max-w-full">
            <ReleaseNotesMarkdown source={update.releaseNotes} />
          </div>
        </div>
      )}

      {update.status === "installing" && (
        <div className="settings-group__state flex-col items-stretch" aria-live="polite">
          <p className="type-caption font-medium text-foreground">
            Downloading version {update.availableVersion ?? "update"}
          </p>
          <Progress
            indeterminate={update.downloadProgress === null}
            label="Update download progress"
            value={update.downloadProgress ?? 0}
          />
          <p className="type-caption text-secondary">
            Work can continue during download. Restart is available after verification.
          </p>
        </div>
      )}

      {isReadyToRestart && platform === "macos" && (
        <p className="settings-group__state text-secondary">
          Updates require a writable Applications folder. macOS may ask for an administrator password. If macOS blocks
          replacement, install the downloaded release manually.
        </p>
      )}

      {update.status === "ready_to_restart" && update.errorMessage && (
        <p className="settings-group__state text-warn" role="status">
          {update.errorMessage}
        </p>
      )}

      {update.errorMessage && update.status !== "ready_to_restart" && (
        <p className="settings-group__state text-danger" role="alert">
          {update.errorMessage}
        </p>
      )}
      {showThanks && <VersionThanks version={update.currentVersion} onClose={() => setShowThanks(false)} />}
    </SettingsGroup>
  );
}
