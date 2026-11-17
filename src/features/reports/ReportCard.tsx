import { Database, EyeOff, FileSpreadsheet, Pin, Presentation } from "lucide-react";
import type { ReportView } from "../../lib/generated/contracts";
import { TruncatedText } from "../../ui/TruncatedText";
import { ReportActions, type ReportActionHandlers } from "./ReportActions";
import { ReportProjectChip } from "./ReportProjectChip";
import {
  formatRelativeUpdated,
  isFileReport,
  reportHandle,
  reportKindLabel,
  reportSharing,
} from "./reportPresentation";
import { ReportThumbnail } from "./ReportThumbnail";
import type { ReportMutation } from "./useReports";

/**
 * One saved report as a card in the grid, after xfalcon-web's report cards.
 *
 * **The title is the control, and the card is its hit area.** The title button
 * carries a stretched pseudo-element (`after:inset-0`) covering the card, so a
 * click anywhere opens the report while there is still exactly one interactive
 * element doing it. Web instead puts `role="button"` on the card and nests the
 * three action buttons inside it — a button inside a button, which leaves a
 * screen reader no path to the inner controls and makes a click on Pin activate
 * both. This gets web's whole-card feel without that.
 *
 */
export function ReportCard({
  report,
  busyAction = null,
  projectName = null,
  onOpen,
  ...handlers
}: {
  report: ReportView;
  /** The project the report was created in, when this device knows it. */
  projectName?: string | null;
  busyAction?: ReportMutation | null;
  onOpen: (report: ReportView) => void;
} & ReportActionHandlers) {
  const pinned = report.pinned;
  const sharing = reportSharing(report);

  return (
    <div className="report-card group relative flex min-w-0 flex-col overflow-hidden">
      {isFileReport(report) ? (
        // A workbook or deck is a file with no page to frame, so its card shows what
        // it is instead of a preview that could never load.
        <div aria-hidden="true" className="report-card__thumbnail report-card__thumbnail--file">
          {report.kind === "workbook" ? (
            <FileSpreadsheet size={32} strokeWidth={1.4} />
          ) : (
            <Presentation size={32} strokeWidth={1.4} />
          )}
          <span className="type-caption text-muted">{reportKindLabel(report.kind)}</span>
        </div>
      ) : (
        <ReportThumbnail
          reportId={report.id}
          updatedAt={report.updated_at_ms}
          thumbnailAt={report.thumbnail_updated_at_ms}
        />
      )}
      <div className="report-card__body">
        {pinned || report.hidden ? (
          <span className="flex items-center gap-1.5">
            {pinned ? (
              <span className="inline-flex items-center gap-1 rounded-pill bg-accent-muted px-2 py-0.5 text-2xs font-medium text-secondary">
                <Pin aria-hidden className="shrink-0 text-secondary" size={10} />
                Pinned
              </span>
            ) : null}
            {report.hidden ? (
              <span className="inline-flex items-center gap-1 rounded-pill bg-surface-alt px-2 py-0.5 text-2xs font-medium text-muted">
                <EyeOff aria-hidden className="shrink-0" size={10} />
                Hidden
              </span>
            ) : null}
          </span>
        ) : null}

        <h3 className="min-w-0">
          {/* ui-extension: the card's title is its click target, and the
            `after:inset-0` overlay makes the whole card press it. A boxed
            control would draw a button where a heading belongs. */}
          <button
            aria-label={`Open ${reportHandle(report)}`}
            className="block w-full cursor-pointer border-0 bg-transparent p-0 text-left type-heading font-bold text-foreground transition-colors group-hover:text-accent after:absolute after:inset-0 after:rounded-[inherit]"
            type="button"
            onClick={() => onOpen(report)}
          >
            <TruncatedText as="span" className="block" text={report.title}>
              {report.title}
            </TruncatedText>
          </button>
        </h3>

        {report.description !== null ? (
          <p className="line-clamp-2 type-label text-secondary">{report.description}</p>
        ) : null}

        {/* One quiet line: when, and who else sees it. */}
        <p className="report-card__meta type-micro text-muted">
          <span>Updated {formatRelativeUpdated(report.updated_at_ms)}</span>
          {sharing !== null ? (
            <>
              <span aria-hidden className="report-card__dot">
                ·
              </span>
              <span>{sharing}</span>
            </>
          ) : null}
        </p>
        {report.data_sources.length > 0 ? (
          <p className="report-card__sources type-micro text-muted">
            <Database aria-hidden size={11} />
            <span className="sr-only">Data from </span>
            <TruncatedText as="span" text={report.data_sources.join(", ")}>
              {report.data_sources.join(", ")}
            </TruncatedText>
          </p>
        ) : null}

        {/* Grid items stretch to their row, so `mt-auto` is what lines the footers
          up across cards whose descriptions run to different lengths. The project
          sits here rather than above the title, so every title starts level; with
          none, the actions keep to the right on their own. */}
        <div className="report-card__footer">
          {projectName !== null ? (
            <span className="report-card__kind type-micro text-muted">
              <ReportProjectChip name={projectName} />
            </span>
          ) : null}
          <ReportActions busyAction={busyAction} report={report} {...handlers} />
        </div>
      </div>
    </div>
  );
}
