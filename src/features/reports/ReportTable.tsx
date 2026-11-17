import type React from "react";
import { useId, useMemo, useRef, useState } from "react";
import {
  ArrowDown,
  ArrowUp,
  ChevronsUpDown,
  Database,
  EyeOff,
  FileSpreadsheet,
  Lock,
  Presentation,
  Users,
} from "lucide-react";
import type { ReportProjectOrigin, ReportView } from "../../lib/generated/contracts";
import { Avatar } from "../../ui/Avatar";
import { Pagination } from "../../ui/Pagination";
import { Select } from "../../ui/Select";
import { Table, TableBody, TableHead, TableRow, Td, Th } from "../../ui/Table";
import { TruncatedText } from "../../ui/TruncatedText";
import { ReportActions, type ReportActionHandlers } from "./ReportActions";
import { ReportProjectChip } from "./ReportProjectChip";
import { canCapturePreview } from "./reportPreviewCapture";
import { formatRelativeUpdated, isFileReport, reportHandle } from "./reportPresentation";
import { ReportThumbnail } from "./ReportThumbnail";
import type { ReportMutation } from "./useReports";

type SortKey = "name" | "owner" | "created" | "updated";
type Sort = { key: SortKey; direction: "ascending" | "descending" };

/** The way a column first sorts: names A to Z, dates newest first. */
const FIRST_DIRECTION: Record<SortKey, Sort["direction"]> = {
  name: "ascending",
  owner: "ascending",
  created: "descending",
  updated: "descending",
};

/** How many rows a page holds, and the choices: a library of hundreds is paged
 * rather than drawn whole. The footer shows once there are more rows than the
 * smallest choice. */
const DEFAULT_PAGE_SIZE = 20;
const PAGE_SIZES = [10, 20, 50, 100] as const;

/**
 * The library as rows: the List view, and the reports below the recent cards.
 *
 * Each row reads left to right as a report does: a small picture and what it
 * is (name, description, where its data comes from, its project), then who owns
 * it, how many pages, who can see it, when it was made and changed, and every
 * action a card offers. A row opens on a click anywhere outside its controls;
 * the name stays the one control that opens it from the keyboard.
 *
 * Name, Owner, Created and Updated sort from their headers: a first press sorts
 * the column its natural way, a second reverses it, and a third gives the order
 * back to the library's own sort. The sort covers every row; pages are cut from
 * the sorted list, 20 at a time unless the reader picks another size.
 */
export function ReportTable({
  reports,
  projectOrigins,
  busyById,
  onOpen,
  actions,
}: {
  reports: ReportView[];
  projectOrigins: ReportProjectOrigin[];
  busyById: Readonly<Record<string, ReportMutation>>;
  onOpen: (report: ReportView) => void;
  actions: ReportActionHandlers;
}) {
  const [sort, setSort] = useState<Sort | null>(null);
  const [pageSize, setPageSize] = useState<number>(DEFAULT_PAGE_SIZE);
  const [requestedPage, setRequestedPage] = useState(1);
  const top = useRef<HTMLDivElement>(null);
  const ordered = useMemo(() => (sort === null ? reports : sorted(reports, sort)), [reports, sort]);
  const pageCount = Math.max(1, Math.ceil(ordered.length / pageSize));
  // A search or a delete can leave fewer pages than the reader was on.
  const page = Math.min(requestedPage, pageCount);
  const rows = useMemo(() => ordered.slice((page - 1) * pageSize, page * pageSize), [ordered, page, pageSize]);
  // The actions column is as wide as its widest row: a table of reports shared
  // with the reader holds two controls a row, not six.
  const actionSlots = useMemo(() => Math.max(2, ...rows.map(reportActionCount)), [rows]);
  const press = (key: SortKey) => {
    setRequestedPage(1);
    setSort((current) =>
      current?.key !== key
        ? { key, direction: FIRST_DIRECTION[key] }
        : current.direction === FIRST_DIRECTION[key]
          ? { key, direction: current.direction === "ascending" ? "descending" : "ascending" }
          : null,
    );
  };
  const goToPage = (next: number) => {
    setRequestedPage(next);
    // A new page starts at its top: bring the table back into view if the reader
    // had scrolled down its previous page.
    const table = top.current;
    if (table !== null && table.getBoundingClientRect().top < 0) table.scrollIntoView?.({ block: "start" });
  };
  const header = (key: SortKey, label: string, className?: string) => (
    <SortableTh
      className={className}
      label={label}
      sort={sort?.key === key ? sort.direction : null}
      onPress={() => press(key)}
    />
  );

  // The same bar above and below the table, over the same state: a long page
  // is paged from either end without scrolling to the other.
  const footer = (placement: "above" | "below") =>
    ordered.length > PAGE_SIZES[0] ? (
      <ReportTableFooter
        count={ordered.length}
        page={page}
        pageCount={pageCount}
        pageSize={pageSize}
        placement={placement}
        shown={rows.length}
        onPageChange={goToPage}
        onPageSizeChange={(size) => {
          setPageSize(size);
          setRequestedPage(1);
        }}
      />
    ) : null;

  return (
    <div ref={top} className="reports-table-wrap">
      {footer("above")}
      <Table className="reports-table" style={{ "--reports-action-slots": actionSlots } as React.CSSProperties}>
        <TableHead>
          <TableRow>
            {header("name", "Name", "reports-col-name")}
            {header("owner", "Owner", "reports-col-owner")}
            <Th className="reports-col-pages">Pages</Th>
            <Th className="reports-col-visibility">Visibility</Th>
            {header("created", "Created", "reports-col-created")}
            {header("updated", "Updated", "reports-col-updated")}
            <Th className="reports-col-actions">Actions</Th>
          </TableRow>
        </TableHead>
        <TableBody>
          {rows.map((report) => (
            <ReportRow
              busyAction={busyById[report.id] ?? null}
              key={report.id}
              projectName={projectOrigins.find((origin) => origin.report_id === report.id)?.project_name ?? null}
              report={report}
              onOpen={onOpen}
              actions={actions}
            />
          ))}
        </TableBody>
      </Table>
      {footer("below")}
    </div>
  );
}

/** Where the reader is in the table, the rows a page holds, and the way to
 * move: "Showing 21–40 of 45", the size dropdown, and Previous and Next. */
function ReportTableFooter({
  count,
  page,
  pageCount,
  pageSize,
  shown,
  placement,
  onPageChange,
  onPageSizeChange,
}: {
  count: number;
  page: number;
  pageCount: number;
  pageSize: number;
  shown: number;
  placement: "above" | "below";
  onPageChange: (page: number) => void;
  onPageSizeChange: (size: number) => void;
}) {
  const sizeLabelId = useId();
  const first = (page - 1) * pageSize + 1;
  return (
    <div className="reports-table-footer" data-placement={placement}>
      <p className="type-caption tabular-nums text-muted">
        Showing {first}–{first + shown - 1} of {count}
      </p>
      <div className="reports-table-footer__controls">
        <span className="reports-page-size">
          <span className="type-caption text-muted" id={sizeLabelId}>
            Rows per page
          </span>
          <Select
            aria-labelledby={sizeLabelId}
            options={PAGE_SIZES.map((size) => ({ value: String(size), label: String(size) }))}
            value={String(pageSize)}
            onChange={(size) => onPageSizeChange(Number(size))}
          />
        </span>
        {pageCount > 1 ? (
          <Pagination
            label={`Pagination ${placement} the table`}
            page={page}
            pageCount={pageCount}
            onPageChange={onPageChange}
          />
        ) : null}
      </div>
    </div>
  );
}

/** A column header that sorts the table, saying how it is sorted now. */
function SortableTh({
  label,
  sort,
  className,
  onPress,
}: {
  label: string;
  sort: Sort["direction"] | null;
  className?: string;
  onPress: () => void;
}) {
  const Icon = sort === "ascending" ? ArrowUp : sort === "descending" ? ArrowDown : ChevronsUpDown;
  return (
    <Th aria-sort={sort ?? "none"} className={className}>
      <button className="reports-sort-header" data-sorted={sort !== null} type="button" onClick={onPress}>
        {label}
        <Icon aria-hidden size={12} />
      </button>
    </Th>
  );
}

function sorted(reports: ReportView[], sort: Sort): ReportView[] {
  const sign = sort.direction === "ascending" ? 1 : -1;
  const compare = (a: ReportView, b: ReportView): number => {
    switch (sort.key) {
      case "name":
        return a.title.localeCompare(b.title, undefined, { sensitivity: "base", numeric: true });
      case "owner":
        return ownerName(a).localeCompare(ownerName(b), undefined, { sensitivity: "base" });
      case "created":
        return Number(a.created_at_ms - b.created_at_ms);
      case "updated":
        return Number(a.updated_at_ms - b.updated_at_ms);
    }
  };
  // Ties keep a stable, repeatable order, so a re-sort never shuffles rows.
  return [...reports].sort((a, b) => sign * compare(a, b) || a.id.localeCompare(b.id));
}

/** How many controls `ReportActions` draws for a report: pin and hide always,
 * share and rename for its manager, a preview for a dashboard's owner, delete
 * when allowed. Kept beside the column it sizes; change it with `ReportActions`. */
function reportActionCount(report: ReportView): number {
  return 2 + Number(report.can_manage) * 2 + Number(canCapturePreview(report)) + Number(report.can_delete);
}

function ownerName(report: ReportView): string {
  return report.owner_display_name ?? (report.is_owner ? "You" : "Unknown owner");
}

function ReportRow({
  report,
  projectName,
  busyAction,
  onOpen,
  actions,
}: {
  report: ReportView;
  projectName: string | null;
  busyAction: ReportMutation | null;
  onOpen: (report: ReportView) => void;
  actions: ReportActionHandlers;
}) {
  const owner = ownerName(report);
  const organization = report.scope === "global";
  const openFromRow = (event: React.MouseEvent<HTMLTableRowElement>) => {
    // A click on a control is that control's; a click on a cell opens the report.
    if ((event.target as HTMLElement).closest("button, a, input, [role='menuitem']")) return;
    onOpen(report);
  };

  return (
    <TableRow interactive className="reports-table-row" onClick={openFromRow}>
      <Td className="reports-col-name">
        <div className="reports-table-name">
          <RowPicture report={report} />
          <div className="reports-table-text">
            <button
              aria-label={`Open ${reportHandle(report)}`}
              className="reports-row-title"
              type="button"
              onClick={() => onOpen(report)}
            >
              <TruncatedText as="span" className="block" text={report.title}>
                {report.title}
              </TruncatedText>
            </button>
            {report.description !== null ? (
              <TruncatedText as="p" className="type-caption text-muted" text={report.description}>
                {report.description}
              </TruncatedText>
            ) : null}
            {report.data_sources.length > 0 || projectName !== null || report.hidden ? (
              <div className="reports-table-tags">
                {report.data_sources.length > 0 ? (
                  <span className="report-tag report-tag--source">
                    <Database aria-hidden size={11} />
                    <span className="sr-only">Data from </span>
                    <TruncatedText as="span" text={report.data_sources.join(", ")}>
                      {report.data_sources.join(", ")}
                    </TruncatedText>
                  </span>
                ) : null}
                {projectName !== null ? <ReportProjectChip name={projectName} /> : null}
                {report.hidden ? (
                  <span className="report-tag">
                    <EyeOff aria-hidden size={11} />
                    Hidden
                  </span>
                ) : null}
              </div>
            ) : null}
          </div>
        </div>
      </Td>
      <Td className="reports-col-owner">
        <span className="reports-table-owner">
          <Avatar size="sm">{initials(owner)}</Avatar>
          <TruncatedText as="span" text={owner}>
            {owner}
          </TruncatedText>
        </span>
      </Td>
      <Td className="reports-col-pages">
        {isFileReport(report) ? "—" : `${report.page_count} ${report.page_count === 1 ? "page" : "pages"}`}
      </Td>
      <Td className="reports-col-visibility">
        <span className="reports-table-visibility">
          {organization ? <Users aria-hidden size={13} /> : <Lock aria-hidden size={13} />}
          {organization ? "Organization" : "Private"}
        </span>
      </Td>
      <Td className="reports-col-created">
        {report.created_at_ms === 0n ? "—" : formatRelativeUpdated(report.created_at_ms)}
      </Td>
      <Td className="reports-col-updated">{formatRelativeUpdated(report.updated_at_ms)}</Td>
      <Td className="reports-col-actions">
        <ReportActions busyAction={busyAction} report={report} {...actions} />
      </Td>
    </TableRow>
  );
}

/** The row's picture: a dashboard's stored image, or a quiet tile naming what
 * the report is. A row never draws a whole dashboard to show it at this size. */
function RowPicture({ report }: { report: ReportView }) {
  if (report.kind === "workbook" || report.kind === "presentation") {
    const Icon = report.kind === "workbook" ? FileSpreadsheet : Presentation;
    return (
      <span aria-hidden className="reports-table-picture reports-table-picture--tile">
        <Icon size={18} strokeWidth={1.5} />
      </span>
    );
  }
  return (
    <ReportThumbnail
      className="reports-table-picture"
      live={false}
      reportId={report.id}
      thumbnailAt={report.thumbnail_updated_at_ms}
      updatedAt={report.updated_at_ms}
    />
  );
}

function initials(name: string): string {
  return (
    name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((part) => part[0])
      .join("")
      .toUpperCase() || "?"
  );
}
