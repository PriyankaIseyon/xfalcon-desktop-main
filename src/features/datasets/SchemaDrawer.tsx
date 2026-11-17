import { useEffect, useRef, useState } from "react";
import { Download } from "lucide-react";
import type { DatasetDetail, DatasetFileView, DatasetView } from "../../lib/generated/contracts";
import { datasetDetail } from "../../lib/native";
import { Badge } from "../../ui/Badge";
import { Button } from "../../ui/Button";
import { Modal } from "../../ui/Modal";
import { SearchField } from "../../ui/SearchField";
import { ColumnTypeBadge } from "./ColumnTypeBadge";
import { Spinner } from "../../ui/Spinner";
import { TruncatedText } from "../../ui/TruncatedText";
import { describeError, formatBytes, formatCount, formatUploaded } from "./datasetPresentation";
import { datasetErrorOf } from "./useDatasets";

/**
 * What one dataset actually holds: its files, their sizes and counts, and each
 * file's inferred columns in the order the workspace returned them.
 *
 * A count the server never recorded reads as "Unknown", not as zero — a table
 * of no rows and a table whose count was never written are different facts, and
 * showing both as `0` tells someone their data is empty.
 */
export interface SchemaDrawerProps {
  dataset: DatasetView | null;
  onClose: () => void;
  onDownload: (dataset: DatasetView, file: DatasetFileView) => Promise<boolean>;
}

export function SchemaDrawer({ dataset, onClose, onDownload }: SchemaDrawerProps) {
  const [detail, setDetail] = useState<DatasetDetail | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [loadRevision, setLoadRevision] = useState(0);
  const [busyFileId, setBusyFileId] = useState<string | null>(null);
  const savingRef = useRef(false);

  useEffect(() => {
    if (dataset === null) return;
    let live = true;
    setDetail(null);
    setProblem(null);
    setBusyFileId(null);
    savingRef.current = false;
    void datasetDetail(dataset.id)
      .then((next) => {
        if (!live) return;
        if (next.dataset.id !== dataset.id) {
          setProblem("The dataset details did not match the selected dataset.");
          return;
        }
        setDetail(next);
      })
      .catch((cause) => {
        if (live) setProblem(describeError(datasetErrorOf(cause).code));
      });
    return () => {
      live = false;
    };
  }, [dataset, loadRevision]);

  if (dataset === null) return null;

  return (
    <Modal open size="lg" dismissible={busyFileId === null} onClose={onClose} title={dataset.display_name}>
      <div className="space-y-4">
        <dl className="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <Meta label="Status" value={<Badge tone="success">{dataset.status}</Badge>} />
          <Meta label="Uploaded" value={formatUploaded(Number(dataset.created_at_ms))} />
          <Meta label="Last updated" value={formatUploaded(Number(dataset.updated_at_ms))} />
        </dl>

        {problem !== null ? (
          <div className="flex items-center justify-between gap-3 rounded-md border border-danger bg-danger-bg px-3 py-2">
            <p className="type-caption text-danger">{problem}</p>
            <Button type="button" variant="secondary" onClick={() => setLoadRevision((current) => current + 1)}>
              Try again
            </Button>
          </div>
        ) : null}

        {detail === null && problem === null ? (
          <div className="flex items-center gap-2">
            <Spinner decorative size={16} />
            <span className="type-caption text-muted">Reading the schema…</span>
          </div>
        ) : null}

        {detail?.files.map((file) => (
          <FileSection
            key={file.file_id}
            file={file}
            busy={busyFileId === file.file_id}
            disabled={!detail.uploads_enabled || busyFileId !== null}
            onDownload={async () => {
              if (savingRef.current) return;
              savingRef.current = true;
              setBusyFileId(file.file_id);
              try {
                await onDownload(dataset, file);
              } finally {
                savingRef.current = false;
                setBusyFileId(null);
              }
            }}
          />
        ))}

        {detail !== null && detail.files.length === 0 ? (
          <p className="type-caption text-muted">This dataset holds no files.</p>
        ) : null}
      </div>
    </Modal>
  );
}

function Meta({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div>
      <dt className="type-caption uppercase tracking-wide text-muted">{label}</dt>
      <dd className="type-body mt-0.5">{value}</dd>
    </div>
  );
}

function FileSection({
  file,
  busy,
  disabled,
  onDownload,
}: {
  file: DatasetFileView;
  busy: boolean;
  disabled: boolean;
  onDownload: () => Promise<void>;
}) {
  const [query, setQuery] = useState("");
  const needle = query.trim().toLocaleLowerCase();
  const columns = file.columns
    .map((column, index) => ({ ...column, index }))
    .filter(
      (column) =>
        column.name.toLocaleLowerCase().includes(needle) || column.inferred_type.toLocaleLowerCase().includes(needle),
    );
  return (
    <section className="rounded-lg border border-hairline bg-card p-3">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="type-label">
            <TruncatedText as="span" className="block" text={file.table_name}>
              {file.table_name}
            </TruncatedText>
          </h3>
          <p className="type-caption text-muted">
            {formatCount(file.row_count === null ? null : Number(file.row_count))} rows ·{" "}
            {formatCount(file.column_count)} columns ·{" "}
            {formatBytes(file.size_bytes === null ? null : Number(file.size_bytes))}
            {file.original_filename === null ? "" : ` · from ${file.original_filename}`}
          </p>
        </div>
        <Button type="button" variant="secondary" disabled={disabled} loading={busy} onClick={() => void onDownload()}>
          {busy ? null : <Download aria-hidden size={14} />}
          {busy ? "Saving…" : "Save a copy"}
        </Button>
      </header>

      {file.columns.length === 0 ? (
        <p className="type-caption mt-2 text-muted">The workspace has not recorded this file's columns.</p>
      ) : (
        <div className="mt-4 space-y-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="type-caption text-muted" role="status">
              {columns.length} of {file.columns.length} columns
            </p>
            <SearchField
              aria-label={`Search columns in ${file.table_name}`}
              placeholder="Search column name or type"
              className="w-full sm:w-72"
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
              onClear={() => setQuery("")}
            />
          </div>
          <div className="schema-columns">
            <div className="schema-columns__heading" aria-hidden="true">
              <span>#</span>
              <span>Column name</span>
              <span>Data type</span>
            </div>
            <ul className="schema-columns__list" aria-label={`Columns in ${file.table_name}`}>
              {columns.map((column) => (
                <li key={`${column.name}-${column.index}`} className="schema-columns__row">
                  <span aria-hidden="true" className="type-micro tabular-nums text-muted">
                    {column.index + 1}
                  </span>
                  <span className="min-w-0 break-words type-label font-medium">{column.name}</span>
                  <ColumnTypeBadge type={column.inferred_type} />
                </li>
              ))}
              {columns.length === 0 ? (
                <li role="presentation" className="p-4 type-caption text-muted">
                  No columns match “{query.trim()}”.
                </li>
              ) : null}
            </ul>
          </div>
        </div>
      )}
    </section>
  );
}
