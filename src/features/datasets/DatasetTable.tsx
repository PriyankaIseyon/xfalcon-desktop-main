import { ArrowDown, ArrowUp, Database, Download, TableProperties, Trash2 } from "lucide-react";
import type { DatasetView } from "../../lib/generated/contracts";
import { IconButton } from "../../ui/IconButton";
import { Table, TableBody, TableHead, TableRow, Td, Th } from "../../ui/Table";
import { Tooltip } from "../../ui/Tooltip";
import { TruncatedText } from "../../ui/TruncatedText";
import {
  formatUploaded,
  formatUploadedExact,
  type DatasetSortDirection,
  type DatasetSortKey,
} from "./datasetPresentation";

/**
 * The catalog, as a table.
 *
 * Row counts are not columns here. They live on one dataset's files, so showing
 * a total would mean asking the workspace for every dataset's detail to draw one
 * list — the schema drawer is where those numbers belong.
 *
 * There is no status column either, for the same kind of reason: this list holds
 * only the datasets the workspace reports as ready
 * (`datasets/catalog.rs::list`), so the column would print one word on every
 * row forever.
 *
 * The row's actions are always visible rather than behind a menu. Three actions
 * on a dense row cost less width than the menu button plus the click it takes
 * to find out what is in it, and a person scanning a list of datasets is
 * usually here to open, save, or remove one.
 */
export interface DatasetTableProps {
  datasets: DatasetView[];
  sortKey: DatasetSortKey;
  sortDirection: DatasetSortDirection;
  onSort: (key: DatasetSortKey) => void;
  onOpen: (dataset: DatasetView) => void;
  onDownload: (dataset: DatasetView) => void;
  onDelete: (dataset: DatasetView) => void;
  /** The row whose file is being resolved for saving. One at a time: two save
   * dialogs for two rows is two dialogs nobody asked for. */
  busyId: string | null;
  downloadsEnabled: boolean;
}

export function DatasetTable({
  datasets,
  sortKey,
  sortDirection,
  onSort,
  onOpen,
  onDownload,
  onDelete,
  busyId,
  downloadsEnabled,
}: DatasetTableProps) {
  return (
    <Table flush className="dataset-library-table">
      <TableHead sticky>
        <TableRow>
          <SortableTh
            className="pl-4"
            column="name"
            label="Name"
            sortKey={sortKey}
            sortDirection={sortDirection}
            onSort={onSort}
          />
          <SortableTh
            column="uploaded"
            label="Uploaded"
            sortKey={sortKey}
            sortDirection={sortDirection}
            onSort={onSort}
          />
          <Th className="pr-4 text-right" scope="col">
            <span className="sr-only">Actions</span>
          </Th>
        </TableRow>
      </TableHead>
      <TableBody>
        {datasets.map((dataset) => (
          <TableRow key={dataset.id} interactive className="group" onClick={() => onOpen(dataset)}>
            {/* `w-full max-w-0` lets the name take every pixel the other two
                columns do not want, and still ellipsize when it runs out. A
                fixed cap truncated names with a thousand pixels of table to
                spare. */}
            <Td className="w-full max-w-0 pl-4 font-medium text-foreground transition-colors group-hover:text-accent">
              <div className="dataset-name-cell">
                <span className="dataset-name-icon" aria-hidden>
                  <Database size={17} strokeWidth={1.6} />
                </span>
                <TruncatedText as="span" className="block min-w-0" text={dataset.display_name}>
                  {dataset.display_name}
                </TruncatedText>
              </div>
            </Td>
            <Td className="whitespace-nowrap tabular-nums text-muted">
              <Tooltip label={formatUploadedExact(Number(dataset.created_at_ms))}>
                <time
                  tabIndex={0}
                  dateTime={
                    Number(dataset.created_at_ms) > 0
                      ? new Date(Number(dataset.created_at_ms)).toISOString()
                      : undefined
                  }
                >
                  {formatUploaded(Number(dataset.created_at_ms))}
                </time>
              </Tooltip>
            </Td>
            <Td className="w-px pr-4">
              {/* Stops the row's own open handler: each of these is a second
                  action on the same surface, and opening the drawer behind a
                  delete confirmation would bury what the person asked for. */}
              {/* Enabled actions remain legible at rest; disabled controls keep their own treatment. */}
              <div
                className="flex items-center justify-end gap-0.5 text-secondary"
                onClick={(event) => event.stopPropagation()}
              >
                <IconButton aria-label={`Schema and files for ${dataset.display_name}`} onClick={() => onOpen(dataset)}>
                  <TableProperties aria-hidden size={15} />
                </IconButton>
                <IconButton
                  aria-label={`Save ${dataset.display_name}`}
                  disabled={!downloadsEnabled || busyId !== null}
                  loading={busyId === dataset.id}
                  onClick={() => onDownload(dataset)}
                >
                  {busyId === dataset.id ? null : <Download aria-hidden size={15} />}
                </IconButton>
                <IconButton
                  aria-label={`Delete ${dataset.display_name}`}
                  variant="danger"
                  onClick={() => onDelete(dataset)}
                >
                  <Trash2 aria-hidden size={15} />
                </IconButton>
              </div>
            </Td>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}

function SortableTh({
  className,
  column,
  label,
  sortKey,
  sortDirection,
  onSort,
}: {
  className?: string;
  column: DatasetSortKey;
  label: string;
  sortKey: DatasetSortKey;
  sortDirection: DatasetSortDirection;
  onSort: (key: DatasetSortKey) => void;
}) {
  const active = sortKey === column;
  return (
    <Th
      className={className}
      scope="col"
      aria-sort={active ? (sortDirection === "asc" ? "ascending" : "descending") : "none"}
    >
      {/* ui-extension: the sort affordance IS the column header. A boxed
          control here would draw a button inside a table head rather than a
          header that can be pressed. */}
      <button
        type="button"
        className="group/sort -mx-1 inline-flex items-center gap-1 rounded px-1 py-0.5 transition-colors hover:text-foreground"
        onClick={() => onSort(column)}
      >
        {label}
        {/* The arrow occupies its width whether or not this column is the one
            sorted, so choosing a different column does not nudge the header
            text sideways. A column that could be sorted says so faintly when
            the pointer is on it. */}
        <span
          aria-hidden
          className={
            active ? "inline-flex opacity-100" : "inline-flex opacity-0 transition-opacity group-hover/sort:opacity-45"
          }
        >
          {active && sortDirection === "asc" ? <ArrowUp size={12} /> : <ArrowDown size={12} />}
        </span>
      </button>
    </Th>
  );
}
