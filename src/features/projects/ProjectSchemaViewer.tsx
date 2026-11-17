import React from "react";
import { Database, Table } from "lucide-react";
import type { ProjectId, ProjectSchemaSummary, ProjectTableSchema } from "../../lib/generated/contracts";
import { projectSchemaSummary } from "../../lib/native";
import { Alert } from "../../ui/Alert";
import { Badge } from "../../ui/Badge";
import { Button } from "../../ui/Button";
import { Skeleton } from "../../ui/Skeleton";
import { TruncatedText } from "../../ui/TruncatedText";
import { describeProjectError } from "./projectFileText";
import { ColumnTypeBadge } from "../datasets/ColumnTypeBadge";

export interface ProjectSchemaViewerProps {
  projectId: ProjectId;
  path: string | null;
}

type SchemaState =
  | { projectId: ProjectId; status: "loading" }
  | { projectId: ProjectId; status: "ready"; summary: ProjectSchemaSummary }
  | { projectId: ProjectId; status: "error"; message: string };

const COUNT_FORMAT = new Intl.NumberFormat();

export function ProjectSchemaViewer({ projectId, path }: ProjectSchemaViewerProps) {
  const [request, setRequest] = React.useState(0);
  const [state, setState] = React.useState<SchemaState>({ projectId, status: "loading" });

  React.useEffect(() => {
    let cancelled = false;
    setState({ projectId, status: "loading" });

    void projectSchemaSummary(projectId).then(
      (summary) => {
        if (!cancelled) setState({ projectId, status: "ready", summary });
      },
      (cause: unknown) => {
        if (!cancelled) {
          setState({
            projectId,
            status: "error",
            message: describeProjectError(cause, "Schemas could not be loaded."),
          });
        }
      },
    );

    return () => {
      cancelled = true;
    };
  }, [projectId, request]);

  // Effects run after paint. Do not show the previous project's schemas for
  // that frame when the active project changes.
  const current: SchemaState = state.projectId === projectId ? state : { projectId, status: "loading" };

  if (current.status === "loading") return <SchemaLoading />;

  if (current.status === "error") {
    return (
      <div className="p-4">
        <Alert
          action={
            <Button size="sm" variant="secondary" onClick={() => setRequest((value) => value + 1)}>
              Try again
            </Button>
          }
          title="Schemas are not available"
          tone="danger"
        >
          <p>{current.message}</p>
        </Alert>
      </div>
    );
  }

  const selectedPath = path?.replaceAll("\\", "/");
  const tables =
    selectedPath === undefined
      ? current.summary.tables
      : current.summary.tables.filter((table) => table.relative_path.replaceAll("\\", "/") === selectedPath);

  if (tables.length === 0) {
    return (
      <div className="p-4">
        <div className="flex items-center gap-2 text-secondary">
          <Database aria-hidden size={16} strokeWidth={1.8} />
          <p className="type-caption">
            {path === null
              ? "No CSV, JSON, or SQLite schemas were found in this project."
              : "No CSV, JSON, or SQLite schema was found for this file."}
          </p>
        </div>
      </div>
    );
  }

  return (
    // Named by the tab it sits in ("Schemas"), which is also why it repeats no
    // heading of its own. "Schemas", not "Datasets": the sidebar's Datasets is
    // the managed catalog; these are detected by scanning this project's files.
    <section aria-label="Schemas" className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4">
      <div className="flex flex-col gap-4">
        {tables.map((table) => (
          <TableSchemaCard key={`${table.relative_path}:${table.name}`} table={table} />
        ))}
      </div>
    </section>
  );
}

function SchemaLoading() {
  return (
    <div aria-busy="true" aria-label="Extracting schemas" className="flex flex-col gap-3 p-4" role="status">
      <div className="flex items-center gap-2">
        <Skeleton className="size-4 rounded-full" />
        <Skeleton className="h-4 w-44" />
      </div>
      <Skeleton className="h-12 w-full" />
      <Skeleton className="h-12 w-full" />
      <span className="sr-only">Extracting schemas…</span>
    </div>
  );
}

/** The file's name without its folder or extension -- what a CSV or JSON
 * table is named after. */
function fileStem(relativePath: string): string {
  const base = relativePath.slice(Math.max(relativePath.lastIndexOf("/"), relativePath.lastIndexOf("\\")) + 1);
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(0, dot) : base;
}

/** Where the table lives, only when the name does not already say it: a
 * SQLite table is named for itself, and a file in a subfolder needs its
 * folder. A top-level CSV named after itself shows its name once. */
function locationHint(table: ProjectTableSchema): string | null {
  const nested = /[/\\]/.test(table.relative_path);
  return nested || table.name !== fileStem(table.relative_path) ? table.relative_path : null;
}

/** Distinct samples in first-seen order: "Easy, Easy, Easy" tells a reader
 * less than "Easy". */
function distinctSamples(values: readonly string[]): string[] {
  return [...new Set(values)];
}

/** One table's schema, always open: the columns are what the tab is for, so
 * hiding them behind a disclosure only added a click. */
function TableSchemaCard({ table }: { table: ProjectTableSchema }) {
  const location = locationHint(table);
  return (
    <section
      aria-label={table.name}
      className="overflow-hidden rounded-xl border border-default bg-surface/40 shadow-e1"
    >
      <header className="flex items-center gap-3 px-4 py-3">
        <span
          aria-hidden
          className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-accent-muted text-accent"
        >
          <Table size={16} strokeWidth={1.8} />
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <TruncatedText className="type-label block font-semibold text-foreground" text={table.name}>
            {table.name}
          </TruncatedText>
          <p className="type-micro flex min-w-0 flex-wrap items-center gap-x-1.5 tabular-nums text-muted">
            <span>{COUNT_FORMAT.format(table.columns.length)} columns</span>
            {table.row_count_estimate == null ? null : (
              <>
                <span aria-hidden>·</span>
                <span>about {COUNT_FORMAT.format(table.row_count_estimate)} rows</span>
              </>
            )}
            {location === null ? null : (
              <>
                <span aria-hidden>·</span>
                <TruncatedText className="block min-w-0 max-w-full font-mono" text={location}>
                  {location}
                </TruncatedText>
              </>
            )}
          </p>
        </div>
        <Badge>{table.format.toLocaleUpperCase()}</Badge>
      </header>

      {table.columns.length === 0 ? (
        <p className="type-caption border-t border-hairline px-4 py-3 text-secondary">This table has no columns.</p>
      ) : (
        <div
          aria-label={`${table.name} columns`}
          className="max-h-[28rem] overflow-auto border-t border-hairline"
          role="region"
          tabIndex={0}
        >
          <table className="type-caption w-full min-w-[32rem] border-collapse text-left">
            <thead className="sticky top-0 z-[1] bg-surface">
              <tr className="type-micro uppercase tracking-wide text-muted">
                <th className="w-[30%] px-4 py-2 font-medium" scope="col">
                  Column
                </th>
                <th className="w-[20%] px-4 py-2 font-medium" scope="col">
                  Type
                </th>
                <th className="px-4 py-2 font-medium" scope="col">
                  Sample values
                </th>
              </tr>
            </thead>
            <tbody>
              {table.columns.map((column, index) => {
                const samples = distinctSamples(column.sample_values);
                return (
                  <tr
                    className="border-t border-hairline transition-colors duration-fast ease-soft hover:bg-hover"
                    key={`${index}:${column.name}`}
                  >
                    <th className="max-w-56 px-4 py-2.5 font-medium text-foreground" scope="row">
                      <TruncatedText className="block max-w-56" text={column.name}>
                        {column.name}
                      </TruncatedText>
                    </th>
                    <td className="px-4 py-2.5">
                      {/* The dataset wrangler's own badge, so a type reads the same everywhere. */}
                      <ColumnTypeBadge type={column.inferred_type} />
                    </td>
                    <td className="px-4 py-2.5">
                      {samples.length === 0 ? (
                        <span className="text-muted">—</span>
                      ) : (
                        <ul aria-label={`${column.name} samples`} className="flex min-w-0 flex-wrap gap-1.5">
                          {samples.map((value, sampleIndex) => (
                            <li
                              className="min-w-0 max-w-[16rem] rounded-md border border-hairline bg-surface px-1.5 py-0.5 font-mono type-micro text-secondary"
                              key={`${sampleIndex}:${value}`}
                            >
                              <TruncatedText className="block" text={value}>
                                {value === "" ? "(empty)" : value}
                              </TruncatedText>
                            </li>
                          ))}
                        </ul>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}
