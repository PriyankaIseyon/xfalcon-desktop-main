import { useEffect, useState } from "react";
import { ArrowDown, ArrowUp, Trash2 } from "lucide-react";
import type { DatasetCastType, DatasetFilterOp, DatasetRecipeStep } from "../../lib/generated/contracts";
import { IconButton } from "../../ui/IconButton";
import { Input } from "../../ui/Field";
import { Select } from "../../ui/Select";
import { describeStep } from "./datasetPresentation";

/**
 * The cleanup a person applies before uploading.
 *
 * Every step is applied by the backend — the same function that produces the
 * uploaded file — so the preview beside this editor is the result, not an
 * approximation of it. That is why a step lands as soon as it is added rather
 * than behind an "apply" button: there is nothing to reconcile.
 *
 * A step the backend refuses is reported by position (`invalidStep`), never
 * dropped. A silently skipped step would upload data that differs from the rows
 * the person just read.
 */

type StepKind = DatasetRecipeStep["kind"];
type StepGroup = "row" | "column" | "value";
const MAX_RECIPE_STEPS = 32;

interface StepOption {
  value: StepKind;
  label: string;
  hint: string;
  group: StepGroup;
  glyph: string;
}

const STEP_KINDS: StepOption[] = [
  { value: "filter", label: "Filter", hint: "Keep rows that match a rule", group: "row", glyph: "▽" },
  { value: "sort", label: "Sort", hint: "Order rows by a column", group: "row", glyph: "↕" },
  { value: "dedup", label: "Drop duplicates", hint: "Remove repeated rows", group: "row", glyph: "≡" },
  { value: "remove_nulls", label: "Drop empties", hint: "Drop rows with an empty cell", group: "row", glyph: "∅" },
  { value: "rename", label: "Rename", hint: "Change a column name", group: "column", glyph: "✎" },
  { value: "cast", label: "Cast type", hint: "Read values as a new type", group: "column", glyph: "⇄" },
  { value: "split", label: "Split", hint: "Split a column by a separator", group: "column", glyph: "⫶" },
  { value: "trim", label: "Trim", hint: "Strip leading and trailing spaces", group: "value", glyph: "⌫" },
  { value: "upper", label: "Uppercase", hint: "Change text to all caps", group: "value", glyph: "A↑" },
  { value: "lower", label: "Lowercase", hint: "Change text to lowercase", group: "value", glyph: "a↓" },
  { value: "replace", label: "Replace", hint: "Find and replace text", group: "value", glyph: "↻" },
  { value: "fill_nulls", label: "Fill empties", hint: "Replace empty cells with a value", group: "value", glyph: "▦" },
];

const STEP_GROUPS: Array<{ value: StepGroup; label: string }> = [
  { value: "row", label: "Row shape" },
  { value: "column", label: "Column shape" },
  { value: "value", label: "Value tweak" },
];

const GROUP_DOT: Record<StepGroup, string> = {
  row: "bg-success",
  column: "bg-accent",
  value: "bg-warn",
};

const GROUP_GLYPH: Record<StepGroup, string> = {
  row: "border-success/30 bg-success-bg text-success",
  column: "border-accent/30 bg-accent-muted text-accent-hover",
  value: "border-warn/30 bg-warn-bg text-warn",
};

const FILTER_OPS: { value: DatasetFilterOp; label: string }[] = [
  { value: "equals", label: "is" },
  { value: "not_equals", label: "is not" },
  { value: "greater_than", label: "is above" },
  { value: "less_than", label: "is below" },
  { value: "at_least", label: "is at least" },
  { value: "at_most", label: "is at most" },
  { value: "contains", label: "contains" },
  { value: "is_null", label: "is empty" },
  { value: "is_not_null", label: "is not empty" },
];

const CAST_TYPES: { value: DatasetCastType; label: string }[] = [
  { value: "text", label: "text" },
  { value: "number", label: "number" },
  { value: "integer", label: "whole number" },
  { value: "boolean", label: "true or false" },
  { value: "date", label: "date" },
];

export interface WrangleEditorProps {
  columns: string[];
  steps: DatasetRecipeStep[];
  onChange: (steps: DatasetRecipeStep[]) => void;
  /** Which step the backend refused, and why, when it refused one. */
  invalidStep: { index: number; message: string } | null;
  disabledReason: string | null;
}

export function WrangleEditor({ columns, steps, onChange, invalidStep, disabledReason }: WrangleEditorProps) {
  const columnsBefore: string[][] = [];
  let finalColumns = columns;
  for (const step of steps) {
    columnsBefore.push(finalColumns);
    finalColumns = columnsAfter(step, finalColumns);
  }

  if (disabledReason !== null) {
    return (
      <section aria-label="Clean up before uploading" className="space-y-1">
        <h3 className="type-label">Clean up</h3>
        <p className="type-caption text-muted">{disabledReason}</p>
      </section>
    );
  }

  const move = (index: number, delta: number) => {
    const target = index + delta;
    if (target < 0 || target >= steps.length) return;
    const next = [...steps];
    const [moved] = next.splice(index, 1);
    next.splice(target, 0, moved);
    onChange(next);
  };

  return (
    <section aria-label="Clean up before uploading" className="space-y-2">
      <h3 className="type-label">Clean up</h3>

      {steps.length === 0 ? (
        <p className="type-caption text-muted">Nothing changed. The file uploads exactly as it is.</p>
      ) : (
        <ol className="space-y-1.5">
          {steps.map((step, index) => (
            <li
              key={index}
              className={
                invalidStep?.index === index
                  ? "rounded-md border border-danger bg-card p-2"
                  : "rounded-md border border-hairline bg-card p-2"
              }
            >
              <div className="flex items-start gap-2">
                <span className="type-caption mt-1 w-5 shrink-0 text-muted">{index + 1}.</span>
                <div className="min-w-0 flex-1 space-y-1.5">
                  <p className="type-body-sm">{describeStep(step)}</p>
                  <StepFields
                    step={step}
                    columns={columnsBefore[index] ?? columns}
                    onChange={(next) => onChange(steps.map((existing, at) => (at === index ? next : existing)))}
                  />
                  {invalidStep?.index === index ? (
                    <p className="type-caption text-danger">{invalidStep.message}</p>
                  ) : null}
                </div>
                <div className="flex shrink-0 items-center gap-0.5">
                  <IconButton
                    aria-label={`Move step ${index + 1} earlier`}
                    disabled={index === 0}
                    onClick={() => move(index, -1)}
                  >
                    <ArrowUp aria-hidden size={14} />
                  </IconButton>
                  <IconButton
                    aria-label={`Move step ${index + 1} later`}
                    disabled={index === steps.length - 1}
                    onClick={() => move(index, 1)}
                  >
                    <ArrowDown aria-hidden size={14} />
                  </IconButton>
                  <IconButton
                    aria-label={`Remove step ${index + 1}`}
                    onClick={() => onChange(steps.filter((_, at) => at !== index))}
                  >
                    <Trash2 aria-hidden size={14} />
                  </IconButton>
                </div>
              </div>
            </li>
          ))}
        </ol>
      )}

      <div className="rounded-xl border border-dashed border-default bg-surface/50 p-3">
        <p className="type-caption mb-3 font-medium uppercase tracking-wide text-muted">
          {steps.length === 0 ? "Add first step" : "Add another step"}
        </p>
        <div className="space-y-3">
          {STEP_GROUPS.map((group) => (
            <fieldset key={group.value} data-step-group={group.value}>
              <legend className="type-caption mb-1.5 flex items-center gap-1.5 font-medium uppercase tracking-wide text-muted">
                <span aria-hidden className={`size-1.5 rounded-full ${GROUP_DOT[group.value]}`} />
                {group.label}
              </legend>
              <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-3">
                {STEP_KINDS.filter((option) => option.group === group.value).map((option) => (
                  <button
                    key={option.value}
                    type="button"
                    aria-label={`${option.label}: ${option.hint}`}
                    disabled={finalColumns.length === 0 || steps.length >= MAX_RECIPE_STEPS}
                    onClick={() => onChange([...steps, blankStep(option.value, finalColumns[0] ?? "")])}
                    className="press group flex min-h-14 min-w-0 cursor-pointer items-start gap-2 rounded-lg border border-default bg-card px-2.5 py-2 text-left shadow-e1 transition-[border-color,background-color,box-shadow] duration-fast hover:border-strong hover:bg-hover hover:shadow-e2 disabled:cursor-default"
                  >
                    <span
                      aria-hidden
                      className={`type-micro mt-0.5 inline-flex size-5 shrink-0 items-center justify-center rounded-full border font-semibold ${GROUP_GLYPH[option.group]}`}
                    >
                      {option.glyph}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="type-body-sm block font-medium text-foreground transition-colors group-hover:text-accent-hover">
                        {option.label}
                      </span>
                      <span className="type-caption block text-muted">{option.hint}</span>
                    </span>
                  </button>
                ))}
              </div>
            </fieldset>
          ))}
        </div>
      </div>
      {steps.length >= MAX_RECIPE_STEPS ? (
        <p className="type-caption text-muted">An upload can carry at most {MAX_RECIPE_STEPS} cleanup steps.</p>
      ) : null}
    </section>
  );
}

/** The controls one step needs, and only those. */
function StepFields({
  step,
  columns,
  onChange,
}: {
  step: DatasetRecipeStep;
  columns: string[];
  onChange: (next: DatasetRecipeStep) => void;
}) {
  const columnOptions = columns.map((name) => ({ value: name, label: name }));
  const [partsText, setPartsText] = useState(step.kind === "split" ? String(step.parts) : "");
  const splitParts = step.kind === "split" ? step.parts : null;
  useEffect(() => {
    if (splitParts !== null) setPartsText(String(splitParts));
  }, [splitParts]);

  if (step.kind === "dedup") return null;

  const columnSelect = (
    <Select
      aria-label="Column"
      className="w-44"
      value={step.column}
      onChange={(value) => onChange({ ...step, column: value })}
      options={columnOptions}
    />
  );

  switch (step.kind) {
    case "filter":
      return (
        <div className="flex flex-wrap items-center gap-2">
          {columnSelect}
          <Select
            aria-label="Comparison"
            className="w-36"
            value={step.op}
            onChange={(value) =>
              onChange({
                ...step,
                op: value as DatasetFilterOp,
                // The null tests read no value, so one left over from a
                // previous comparison would be refused rather than ignored.
                value: value === "is_null" || value === "is_not_null" ? null : (step.value ?? ""),
              })
            }
            options={FILTER_OPS}
          />
          {step.op === "is_null" || step.op === "is_not_null" ? null : (
            <Input
              aria-label="Value"
              className="w-40"
              value={step.value ?? ""}
              onChange={(event) => onChange({ ...step, value: event.currentTarget.value })}
            />
          )}
        </div>
      );
    case "sort":
      return (
        <div className="flex flex-wrap items-center gap-2">
          {columnSelect}
          <Select
            aria-label="Direction"
            className="w-36"
            value={step.descending ? "desc" : "asc"}
            onChange={(value) => onChange({ ...step, descending: value === "desc" })}
            options={[
              { value: "asc", label: "smallest first" },
              { value: "desc", label: "largest first" },
            ]}
          />
        </div>
      );
    case "rename":
      return (
        <div className="flex flex-wrap items-center gap-2">
          {columnSelect}
          <Input
            aria-label="New name"
            className="w-40"
            value={step.new_name}
            onChange={(event) => onChange({ ...step, new_name: event.currentTarget.value })}
          />
        </div>
      );
    case "cast":
      return (
        <div className="flex flex-wrap items-center gap-2">
          {columnSelect}
          <Select
            aria-label="Read as"
            className="w-40"
            value={step.cast_to}
            onChange={(value) => onChange({ ...step, cast_to: value as DatasetCastType })}
            options={CAST_TYPES}
          />
        </div>
      );
    case "replace":
      return (
        <div className="flex flex-wrap items-center gap-2">
          {columnSelect}
          <Input
            aria-label="Find"
            className="w-32"
            value={step.find}
            onChange={(event) => onChange({ ...step, find: event.currentTarget.value })}
          />
          <Input
            aria-label="Replace with"
            className="w-32"
            value={step.replace_with}
            onChange={(event) => onChange({ ...step, replace_with: event.currentTarget.value })}
          />
        </div>
      );
    case "split":
      return (
        <div className="flex flex-wrap items-center gap-2">
          {columnSelect}
          <Input
            aria-label="Separator"
            className="w-24"
            value={step.delimiter}
            onChange={(event) => onChange({ ...step, delimiter: event.currentTarget.value })}
          />
          <Input
            aria-label="Parts"
            className="w-20"
            inputMode="numeric"
            value={partsText}
            aria-invalid={!validParts(partsText) || undefined}
            onBlur={() => {
              if (!validParts(partsText)) setPartsText(String(step.parts));
            }}
            onChange={(event) => {
              const raw = event.currentTarget.value;
              setPartsText(raw);
              if (validParts(raw)) onChange({ ...step, parts: Number(raw) });
            }}
          />
        </div>
      );
    case "fill_nulls":
      return (
        <div className="flex flex-wrap items-center gap-2">
          {columnSelect}
          <Input
            aria-label="Fill with"
            className="w-32"
            value={step.value}
            onChange={(event) => onChange({ ...step, value: event.currentTarget.value })}
          />
        </div>
      );
    default:
      return <div className="flex flex-wrap items-center gap-2">{columnSelect}</div>;
  }
}

function validParts(raw: string): boolean {
  return /^\d+$/.test(raw) && Number(raw) >= 1 && Number(raw) <= 32;
}

function blankStep(kind: StepKind, column: string): DatasetRecipeStep {
  switch (kind) {
    case "filter":
      return { kind, column, op: "equals", value: "" };
    case "sort":
      return { kind, column, descending: false };
    case "rename":
      return { kind, column, new_name: "" };
    case "cast":
      return { kind, column, cast_to: "text" };
    case "replace":
      return { kind, column, find: "", replace_with: "" };
    case "split":
      return { kind, column, delimiter: ",", parts: 2 };
    case "dedup":
      return { kind };
    case "fill_nulls":
      return { kind, column, value: "" };
    default:
      return { kind, column } as DatasetRecipeStep;
  }
}

function columnsAfter(step: DatasetRecipeStep, columns: string[]): string[] {
  if (step.kind === "rename") {
    const nextName = step.new_name.trim();
    return columns.map((name) => (name === step.column ? nextName : name));
  }
  if (step.kind !== "split") return columns;
  const index = columns.indexOf(step.column);
  if (index < 0) return columns;
  const taken = new Set(columns.map((name) => name.toLocaleLowerCase()));
  const created: string[] = [];
  for (let part = 1; part <= Math.min(32, step.parts); part += 1) {
    let candidate = `${step.column}_${part}`;
    let suffix = 2;
    while (taken.has(candidate.toLocaleLowerCase())) {
      candidate = `${step.column}_${part}_${suffix}`;
      suffix += 1;
    }
    taken.add(candidate.toLocaleLowerCase());
    created.push(candidate);
  }
  return [...columns.slice(0, index + 1), ...created, ...columns.slice(index + 1)];
}
