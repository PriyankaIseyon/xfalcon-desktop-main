import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { diagnosticsBundleGenerate, documentExportSave } from "../../lib/native";
import { X } from "lucide-react";
import { Alert } from "../../ui/Alert";
import { SettingsButton, SettingsIconButton } from "./SettingsControls";
import { SettingsGroup, SettingsRow } from "./SettingsLayout";
import { ReportSecurityIssueRow } from "./ReportSecurityIssueRow";
import { getUpdateSnapshot, subscribeUpdate } from "./updateStore";

/** Success is confirmation, not a problem to act on — it clears on its own. */
const SUCCESS_DISMISS_MS = 15_000;

/** Support: the diagnostics export and where to get help, as a block inside ApplicationSection. */
export function DiagnosticsSection() {
  const [isExporting, setIsExporting] = useState(false);
  const [feedback, setFeedback] = useState<{ type: "success" | "error"; message: string } | null>(null);
  const exportingRef = useRef(false);

  useEffect(() => {
    if (feedback?.type !== "success") return undefined;
    const handle = window.setTimeout(() => setFeedback(null), SUCCESS_DISMISS_MS);
    return () => window.clearTimeout(handle);
  }, [feedback]);

  const handleExportDiagnostics = async () => {
    if (exportingRef.current) return;
    exportingRef.current = true;
    setIsExporting(true);
    setFeedback(null);
    try {
      const bundle = await diagnosticsBundleGenerate();
      const result = await documentExportSave(bundle.suggested_filename, bundle.content, "markdown");

      if (result.saved && result.file_name) {
        setFeedback({
          type: "success",
          message: `Diagnostics saved to ${result.file_name}`,
        });
      }
      // If cancelled, leave feedback empty/silent as per design
    } catch (err) {
      setFeedback({
        type: "error",
        message: err instanceof Error ? err.message : "Failed to generate diagnostics bundle",
      });
    } finally {
      exportingRef.current = false;
      setIsExporting(false);
    }
  };

  // The row names the build in the report. The version loads asynchronously, so
  // until it has, the row sends no version rather than the placeholder.
  const update = useSyncExternalStore(subscribeUpdate, getUpdateSnapshot, getUpdateSnapshot);
  const applicationVersion = update.currentVersion === "…" ? "" : update.currentVersion;

  return (
    <SettingsGroup id="settings-support" aria-label="Support">
      <SettingsRow label="Diagnostics bundle" hint="Saved locally. No uploads or network requests.">
        <SettingsButton loading={isExporting} wide onClick={() => void handleExportDiagnostics()}>
          {isExporting ? "Generating…" : "Export diagnostics report"}
        </SettingsButton>
      </SettingsRow>

      {feedback ? (
        <Alert
          className="mx-4 mb-4"
          tone={feedback.type === "success" ? "success" : "danger"}
          title={feedback.type === "success" ? "Diagnostics saved" : "Diagnostics could not be saved"}
          action={
            <span className="flex items-center gap-1">
              {feedback.type === "error" ? (
                <SettingsButton onClick={() => void handleExportDiagnostics()}>Try again</SettingsButton>
              ) : null}
              <SettingsIconButton
                aria-label="Dismiss message"
                icon={<X aria-hidden />}
                onClick={() => setFeedback(null)}
              />
            </span>
          }
        >
          {feedback.message}
        </Alert>
      ) : null}
      <ReportSecurityIssueRow applicationVersion={applicationVersion} />
    </SettingsGroup>
  );
}
