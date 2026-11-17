import { useRef, useState } from "react";
import { RotateCcw } from "lucide-react";
import { appReset } from "../../lib/native";
import { AlertDialog } from "../../ui/AlertDialog";
import { Field, Input } from "../../ui/Field";
import { SettingsButton } from "./SettingsControls";
import { SettingsGroup, SettingsRow } from "./SettingsLayout";

const RESET_CONFIRMATION = "RESET";
const RESET_DESCRIPTION =
  "This erases everything xFalcon has stored on this device and restarts it as a fresh installation. This cannot be undone.";
const RESET_IN_PROGRESS = "Resetting xFalcon… The app will close and reopen in a moment.";
/** Used when the rejection carries no message: the cause, and so what was removed, is unknown. */
const RESET_FAILURE = "xFalcon could not start the reset. Try again.";

/** Reset xFalcon, as a block inside ApplicationSection (no box of its own). */
export function DataResetSection() {
  const [resetOpen, setResetOpen] = useState(false);
  const [resetText, setResetText] = useState("");
  const [resetBusy, setResetBusy] = useState(false);
  const [resetError, setResetError] = useState<string | null>(null);
  // A ref, not state: a second confirm in the same tick must not start a second
  // operation before the first re-renders.
  const resettingRef = useRef(false);

  const closeDialog = () => {
    setResetOpen(false);
    setResetText("");
    setResetError(null);
  };

  const resetXFalcon = async () => {
    if (resettingRef.current) return;
    resettingRef.current = true;
    setResetBusy(true);
    setResetError(null);
    try {
      await appReset();
      // Resolved means the reset has started and the app is closing. The dialog
      // stays busy, and the ref stays set, until the process exits.
    } catch (error) {
      resettingRef.current = false;
      setResetBusy(false);
      setResetError(typeof error === "string" && error.trim() !== "" ? error : RESET_FAILURE);
    }
  };

  return (
    <SettingsGroup id="settings-reset" aria-label="Data and reset">
      <SettingsRow
        label="Reset xFalcon"
        hint="Erases everything xFalcon has stored on this device and starts again, as if it had just been installed."
      >
        <SettingsButton variant="danger" icon={<RotateCcw aria-hidden />} onClick={() => setResetOpen(true)}>
          Reset xFalcon
        </SettingsButton>
      </SettingsRow>

      <AlertDialog
        busy={resetBusy}
        confirmDisabled={resetText !== RESET_CONFIRMATION}
        confirmLabel="Reset xFalcon"
        description={resetBusy ? RESET_IN_PROGRESS : RESET_DESCRIPTION}
        destructive
        error={resetError}
        open={resetOpen}
        title="Reset xFalcon?"
        onCancel={closeDialog}
        onConfirm={() => void resetXFalcon()}
      >
        <p className="type-body font-semibold text-foreground">What will be erased</p>
        <ul className="type-body text-secondary list-disc pl-5 space-y-1">
          <li>Your sign-in on this device — you will be signed out and have to sign in again.</li>
          <li>Cached conversations, trusted projects and the file history xFalcon keeps for them.</li>
          <li>Saved API keys, provider credentials and MCP server connections.</li>
          <li>Imported skills, settings and preferences.</li>
          <li>Logs, the local security audit log, diagnostics and temporary upload files.</li>
        </ul>
        <p className="type-body font-semibold text-foreground">What stays</p>
        <ul className="type-body text-secondary list-disc pl-5 space-y-1">
          <li>Your account and data on the xFalcon server.</li>
          <li>Files and folders on your computer, including the ones you opened as projects.</li>
          <li>Documents and reports you exported or saved elsewhere.</li>
        </ul>
        <p className="type-body text-secondary">
          xFalcon will close and reopen on its own. Other apps on this device are not affected.
        </p>
        <Field label="Type RESET to confirm">
          <Input
            autoComplete="off"
            spellCheck={false}
            value={resetText}
            onChange={(event) => setResetText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter" || resetText !== RESET_CONFIRMATION) return;
              event.preventDefault();
              void resetXFalcon();
            }}
          />
        </Field>
      </AlertDialog>
    </SettingsGroup>
  );
}
