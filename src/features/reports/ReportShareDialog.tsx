import type { ReportScope, ReportView } from "../../lib/generated/contracts";
import { AlertDialog } from "../../ui/AlertDialog";

/**
 * The one decision behind "Share": who can open this report next.
 *
 * Both directions ask first. Sharing gives the whole organization a report and,
 * because a refresh runs under whoever asks for it, lets them re-run its queries
 * under their own grants. Making it private takes it away from everyone who had
 * it. Neither makes a public link: DA has no such thing.
 *
 * `report` is the report the dialog is about, or `null` while it is closed. The
 * target is derived from the report's current audience, so the dialog never
 * offers the state the report is already in.
 */
export function ReportShareDialog({
  report,
  busy,
  error,
  onCancel,
  onConfirm,
}: {
  report: ReportView | null;
  busy: boolean;
  error: string | null;
  onCancel: () => void;
  onConfirm: (report: ReportView, scope: ReportScope) => void;
}) {
  const target: ReportScope = report?.scope === "global" ? "user" : "global";
  const name = report === null ? "this report" : `“${report.title}”`;
  return (
    <AlertDialog
      busy={busy}
      confirmLabel={target === "global" ? "Share with organization" : "Make private"}
      description={
        target === "global"
          ? `People in your organization who have report access will be able to open ${name} and refresh it, which re-runs its queries under their own access. This does not create a public link.`
          : `Only you will be able to open ${name}. People who could open it will lose access.`
      }
      error={error}
      open={report !== null}
      title={target === "global" ? "Share with your organization?" : "Make this report private?"}
      onCancel={onCancel}
      onConfirm={() => {
        if (report !== null && !busy) onConfirm(report, target);
      }}
    />
  );
}
