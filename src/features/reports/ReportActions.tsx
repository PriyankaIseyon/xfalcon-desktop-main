import { Eye, EyeOff, ImageUp, Pencil, Pin, PinOff, Share2, Trash2 } from "lucide-react";
import type { ReportView } from "../../lib/generated/contracts";
import { IconButton } from "../../ui/IconButton";
import { canCapturePreview } from "./reportPreviewCapture";
import { reportHandle } from "./reportPresentation";
import type { ReportMutation } from "./useReports";

export type ReportActionHandlers = {
  onTogglePin: (report: ReportView) => void;
  onToggleHidden: (report: ReportView) => void;
  onShare: (report: ReportView) => void;
  onRename: (report: ReportView) => void;
  onDelete: (report: ReportView) => void;
  /** Makes the dashboard's card image again. Offered to its owner only. */
  onRegeneratePreview: (report: ReportView) => void;
};

/**
 * Pin, hide, share, preview, rename, delete — the same controls in the card grid and in
 * the table, from one place so the two cannot drift in behaviour or in label.
 * Pin and hide are the reader's own and always offered. Sharing, rename and
 * delete are the owner's, and are absent for anyone else: a reader has no use
 * for a control they can never press, and who owns the report is already on
 * its card and its row.
 *
 * Each label carries `reportHandle`, so three controls repeated across a grid
 * and a table still resolve to distinct accessible names even when two reports
 * share a title. The visible tooltip stays the bare verb: it appears next to
 * the report it belongs to, so it does not need to repeat it.
 *
 * `relative z-10` is not decoration. A card stretches its title button's hit
 * area across the whole card, and these controls sit inside that area; without
 * a stacking position they would be underneath it and a click on Pin would open
 * the report instead.
 */
export function ReportActions({
  report,
  busyAction = null,
  onTogglePin,
  onToggleHidden,
  onShare,
  onRename,
  onDelete,
  onRegeneratePreview,
}: { report: ReportView; busyAction?: ReportMutation | null } & ReportActionHandlers) {
  const pinned = report.pinned;
  const hidden = report.hidden;
  const handle = reportHandle(report);
  const busy = busyAction !== null;

  return (
    <span className="relative z-10 inline-flex items-center gap-1">
      <IconButton
        aria-label={`${pinned ? "Unpin" : "Pin"} ${handle}`}
        data-pin-for={report.id}
        disabled={busy}
        loading={busyAction === "pin"}
        size="sm"
        tooltip={pinned ? "Unpin" : "Pin"}
        type="button"
        onClick={() => onTogglePin(report)}
      >
        {pinned ? <PinOff aria-hidden size={14} /> : <Pin aria-hidden size={14} />}
      </IconButton>
      <IconButton
        aria-label={`${hidden ? "Show" : "Hide"} ${handle}`}
        data-hide-for={report.id}
        disabled={busy}
        loading={busyAction === "hide"}
        size="sm"
        tooltip={hidden ? "Show in my list" : "Hide from my list"}
        type="button"
        onClick={() => onToggleHidden(report)}
      >
        {hidden ? <Eye aria-hidden size={14} /> : <EyeOff aria-hidden size={14} />}
      </IconButton>
      {report.can_manage ? (
        <IconButton
          aria-label={`Share ${handle}`}
          disabled={busy}
          loading={busyAction === "share"}
          size="sm"
          tooltip={report.scope === "global" ? "Sharing: Organization" : "Sharing: Private"}
          type="button"
          onClick={() => onShare(report)}
        >
          <Share2 aria-hidden size={14} />
        </IconButton>
      ) : null}
      {/* The image is the owner's to make, and only a dashboard has a page to
          draw, so it is absent for everyone else, like sharing. */}
      {canCapturePreview(report) ? (
        <IconButton
          aria-label={`${report.thumbnail_updated_at_ms === null ? "Create preview for" : "Update preview of"} ${handle}`}
          disabled={busy}
          loading={busyAction === "preview"}
          size="sm"
          tooltip={report.thumbnail_updated_at_ms === null ? "Create preview" : "Update preview"}
          type="button"
          onClick={() => onRegeneratePreview(report)}
        >
          <ImageUp aria-hidden size={14} />
        </IconButton>
      ) : null}
      {report.can_manage ? (
        <IconButton
          aria-label={`Rename ${handle}`}
          disabled={busy}
          loading={busyAction === "rename"}
          size="sm"
          tooltip="Rename"
          type="button"
          onClick={() => onRename(report)}
        >
          <Pencil aria-hidden size={14} />
        </IconButton>
      ) : null}
      {report.can_delete ? (
        <IconButton
          aria-label={`Delete ${handle}`}
          disabled={busy}
          loading={busyAction === "delete"}
          size="sm"
          tooltip="Delete"
          variant="danger"
          type="button"
          onClick={() => onDelete(report)}
        >
          <Trash2 aria-hidden size={14} />
        </IconButton>
      ) : null}
    </span>
  );
}
