/**
 * Reading a number out of text (spec: locale-formatting).
 *
 * A chat table's cells are strings quoted from a warehouse or written by a
 * model, so a measure arrives as `1,234.56`, `1.234,56`, `-$1,200`, `5,00 €` or
 * `1 234,5`. Reading is about the DATA, never the machine: the same cell reads
 * as the same number on every laptop. (Showing a number is the opposite job and
 * follows the host locale; that lives in `formatNumber.ts`.)
 *
 * One grammar both classifies a column as a measure and reads its values. Two
 * separate checks — a pattern to classify, a stripper to read — is how
 * `-$1,200` once passed the classifier and then read as nothing, so the chart
 * drew a gap where a loss belonged. Here a cell that classifies always reads.
 *
 * # How a cell is read
 *
 * Optional sign (`-`, `+`, `−`), optional currency symbol before or after, the
 * digits with their separators, optional `%`. Nothing else: no exponent, no
 * `0x10`, no `3.5M`, no accounting parentheses. A refused cell makes its whole
 * column a non-measure, because a refusal is visible and a wrong reading is not.
 *
 * The digits are read by the arrangement of their separators:
 *
 * - Both `,` and `.` present: the last one is the decimal, the other groups.
 * - One mark repeated (`1,234,567`): it groups; a decimal cannot repeat.
 * - Spaces, thin spaces and apostrophes only ever group (`1 234,5`, `1'234.5`).
 * - One mark once, with a tail that is not three digits (`1,5`, `1.50`): decimal.
 * - One mark once, with a three-digit tail (`1,234`): AMBIGUOUS — 1234 or 1.234.
 *   Unless the head cannot be a group (`0,500`, `1234,567`), which makes it a
 *   decimal.
 *
 * An ambiguous cell takes the convention of its own column's unambiguous cells,
 * then of the rest of the TABLE (one source formats one table one way). A
 * column with nothing to go on at all is settled by which mark its ambiguous
 * cells use:
 *
 * - `d,ddd` (`$1,200 / $3,400 / $12,500`) reads as comma-groups. English
 *   thousands is overwhelmingly the likelier reading, it is how the SQL engines
 *   and models that produce these tables write, and refusing it would stop the
 *   commonest table there is from charting.
 * - `d.ddd` (`4.064 / -810.553`) is refused. It is as likely a German column of
 *   thousands as an English column of three-decimal values, the two readings
 *   differ by a factor of a thousand, and a refused column is visible where a
 *   wrong chart is not.
 */

/** The character a column writes its decimal point with. */
export type DecimalMark = "." | ",";

/** What a numeric column was resolved to. Absent (null) means "not a measure". */
export interface NumberFormat {
  readonly decimal: DecimalMark;
}

const CURRENCY = "$€£¥₹₩₽₺₪₫฿₱₦";
const SIGN = "[-+\\u2212]";

/**
 * The whole cell. Groups 1 and 3 are the sign before and after a leading
 * currency mark (at most one may be present), 2 and 5 are currency marks (at
 * most one), 4 is the digits-and-separators body. A body
 * starts and ends on a digit so a stray separator never survives.
 */
const CELL = new RegExp(
  `^(${SIGN})?(?:([${CURRENCY}])\\s?)?(${SIGN})?` +
    `(\\d(?:[\\d ,.'\\u2019\\u00A0\\u202F\\u2009]*\\d)?)` +
    `(?:\\s?([${CURRENCY}]))?(?:\\s?(%))?$`,
  "u",
);

const SPACE_LIKE = new Set([" ", "'", "’", " ", " ", " "]);

type Shape =
  /** No decimal mark: reads the same under every convention, proves nothing. */
  | { kind: "plain"; value: number }
  /** The separators alone settle it, and they name the column's convention. */
  | { kind: "definite"; value: number; decimal: DecimalMark }
  /** One mark and a three-digit tail: `head` + `tail` is a group, `head.tail` a decimal. */
  | { kind: "ambiguous"; mark: DecimalMark; negative: boolean; head: string; tail: string };

function isMark(character: string): character is DecimalMark {
  return character === "." || character === ",";
}

function other(mark: DecimalMark): DecimalMark {
  return mark === "." ? "," : ".";
}

function split(body: string): { runs: string[]; seps: string[] } | null {
  const runs: string[] = [];
  const seps: string[] = [];
  let run = "";
  for (const character of body) {
    if (character >= "0" && character <= "9") {
      run += character;
      continue;
    }
    // Two separators in a row (`1,,2`, `1, 234`) is not a number.
    if (run === "") return null;
    runs.push(run);
    seps.push(character);
    run = "";
  }
  if (run === "") return null;
  runs.push(run);
  return { runs, seps };
}

/**
 * Whether these digit runs are the integer part of a grouped number: the usual
 * 1-3 digit head with groups of three, or the Indian 2-2-3 pattern
 * (`12,34,567`). A head that starts with a zero is not a group — `0,500` is a
 * decimal, not "zero thousand five hundred".
 */
function isGrouped(runs: readonly string[]): boolean {
  if (runs.length === 1) return true;
  const head = runs[0] as string;
  if (head.length > 3 || head.startsWith("0")) return false;
  const rest = runs.slice(1);
  if (rest.every((run) => run.length === 3)) return true;
  const last = rest[rest.length - 1] as string;
  const middle = rest.slice(0, -1);
  return head.length <= 2 && last.length === 3 && middle.every((run) => run.length === 2);
}

function toNumber(negative: boolean, whole: string, fraction: string | null): number | null {
  const value = Number(fraction === null ? whole : `${whole}.${fraction}`);
  if (!Number.isFinite(value)) return null;
  // `-0` is a value nobody wrote; it must not print or compare as one.
  return value === 0 ? 0 : negative ? -value : value;
}

function classifyBody(body: string, negative: boolean): Shape | null {
  const parts = split(body);
  if (parts === null) return null;
  const { runs, seps } = parts;
  if (seps.length === 0) {
    const value = toNumber(negative, runs[0] as string, null);
    return value === null ? null : { kind: "plain", value };
  }

  const marks = seps.filter(isMark);
  const spaces = seps.filter((separator) => SPACE_LIKE.has(separator));
  const distinctMarks = new Set(marks);
  const distinctSpaces = new Set(spaces);
  // Mixing two kinds of space-like grouping in one number is not a format.
  if (distinctSpaces.size > 1) return null;

  const last = seps[seps.length - 1] as string;
  const definite = (decimal: DecimalMark, whole: readonly string[], fraction: string | null): Shape | null => {
    if (!isGrouped(whole)) return null;
    const value = toNumber(negative, whole.join(""), fraction);
    return value === null ? null : { kind: "definite", value, decimal };
  };

  if (marks.length === 0) {
    // Only spaces or apostrophes: they group, and the number has no fraction.
    if (!isGrouped(runs)) return null;
    const value = toNumber(negative, runs.join(""), null);
    return value === null ? null : { kind: "plain", value };
  }

  if (distinctMarks.size === 2) {
    // Both marks: the last one is the decimal and must stand alone, the other
    // groups the integer part. Spaces on top of that would be a third system.
    if (!isMark(last) || spaces.length > 0) return null;
    if (marks.filter((mark) => mark === last).length !== 1) return null;
    return definite(last, runs.slice(0, -1), runs[runs.length - 1] as string);
  }

  const mark = marks[0] as DecimalMark;
  if (spaces.length > 0) {
    // A space already groups, so the one mark can only be the final decimal.
    if (marks.length !== 1 || last !== mark) return null;
    return definite(mark, runs.slice(0, -1), runs[runs.length - 1] as string);
  }

  if (marks.length >= 2) {
    // A decimal cannot repeat, so the mark groups and the other one is the decimal.
    return definite(other(mark), runs, null);
  }

  const head = runs[0] as string;
  const tail = runs[1] as string;
  if (tail.length !== 3 || head.length > 3 || head.startsWith("0")) {
    // `1,5`, `1.50`, `0,500`, `1234,567`: a group is a nonzero head of at most
    // three digits followed by exactly three, so this can only be a decimal.
    // `012,5` is neither a group nor a number anyone writes.
    if (head.length > 1 && head.startsWith("0")) return null;
    return definite(mark, [head], tail);
  }
  return { kind: "ambiguous", mark, negative, head, tail };
}

/** `null` when the text is not a number under the grammar above. */
function classifyCell(text: string): Shape | null {
  const match = CELL.exec(text.trim());
  if (match === null) return null;
  const [, outerSign, leadingCurrency, innerSign, body, trailingCurrency] = match;
  // One sign, one currency mark: `-$-5` and `$5€` are not numbers.
  if (outerSign !== undefined && innerSign !== undefined) return null;
  if (leadingCurrency !== undefined && trailingCurrency !== undefined) return null;
  const sign = outerSign ?? innerSign;
  const negative = sign === "-" || sign === "−";
  return classifyBody(body as string, negative);
}

function isBlank(cell: string | null | undefined): boolean {
  return cell === null || cell === undefined || cell.trim() === "";
}

interface ColumnState {
  /** The convention this column's own unambiguous cells name, if any. */
  evidence: DecimalMark | null;
  /** Whether any ambiguous cell in the column is written with a dot. */
  dotAmbiguous: boolean;
}

/** The column's own evidence, or `null` for a column that is not a measure at all. */
function inspectColumn(cells: readonly (string | null | undefined)[]): ColumnState | null {
  let evidence: DecimalMark | null = null;
  let dotAmbiguous = false;
  let seen = false;
  for (const cell of cells) {
    if (isBlank(cell)) continue;
    const shape = classifyCell(cell as string);
    if (shape === null) return null;
    if (shape.kind === "definite") {
      // Disagreement between a column's own unambiguous cells is a broken
      // source; refusing the column says so, a majority vote would hide it.
      if (evidence !== null && evidence !== shape.decimal) return null;
      evidence = shape.decimal;
    } else if (shape.kind === "ambiguous" && shape.mark === ".") {
      dotAmbiguous = true;
    }
    seen = true;
  }
  return seen ? { evidence, dotAmbiguous } : null;
}

/**
 * Which columns of one table are measures, and how each writes its decimal.
 *
 * `columns` is column-major (one array of cells per column). `excluded` marks
 * columns that must not be measures whatever they hold (identifiers) and that
 * contribute no evidence. A blank cell is ignored; a cell that is not a number
 * keeps its whole column textual.
 */
export function resolveNumberFormats(
  columns: readonly (readonly (string | null | undefined)[])[],
  excluded: readonly boolean[] = [],
): (NumberFormat | null)[] {
  const states = columns.map((cells, index) => (excluded[index] === true ? null : inspectColumn(cells)));

  // What the rest of the table says, used only by a column with nothing to go
  // on itself. Columns that disagree with each other leave the table silent,
  // and such a column falls back to the default below.
  let table: DecimalMark | null = null;
  let tableConflict = false;
  for (const state of states) {
    if (state === null || state.evidence === null) continue;
    if (table !== null && table !== state.evidence) tableConflict = true;
    table = state.evidence;
  }
  const tableDecimal = tableConflict ? null : table;

  return states.map((state) => {
    if (state === null) return null;
    const decimal = state.evidence ?? tableDecimal;
    if (decimal !== null) return { decimal };
    // Nothing decides: see the header for why `d,ddd` defaults and `d.ddd` does not.
    return state.dotAmbiguous ? null : { decimal: "." };
  });
}

/**
 * One cell under a resolved column format, or `null` when it is not a number
 * there. A cell whose own separators contradict the format reads as `null`
 * instead of being forced into it.
 */
export function readNumber(cell: string | number | boolean | null | undefined, format: NumberFormat): number | null {
  if (typeof cell === "number") return Number.isFinite(cell) ? cell : null;
  if (typeof cell !== "string") return null;
  const shape = classifyCell(cell);
  if (shape === null) return null;
  if (shape.kind === "plain") return shape.value;
  if (shape.kind === "definite") return shape.decimal === format.decimal ? shape.value : null;
  // The mark is this column's decimal point, or it groups thousands.
  const asDecimal = shape.mark === format.decimal;
  return toNumber(shape.negative, asDecimal ? shape.head : shape.head + shape.tail, asDecimal ? shape.tail : null);
}
