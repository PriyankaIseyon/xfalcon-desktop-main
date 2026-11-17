export type CronValidation = { valid: true; description: string } | { valid: false; error: string };

const FIELDS = [
  { name: "Minute", min: 0, max: 59 },
  { name: "Hour", min: 0, max: 23 },
  { name: "Day of month", min: 1, max: 31 },
  { name: "Month", min: 1, max: 12 },
  { name: "Day of week", min: 0, max: 7 },
];
const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];
const MAX_DAYS = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/** Numeric, five-field Unix cron. Steps are anchored to each field or range,
 * not elapsed durations. No parsing of Quartz extensions or natural language. */
export function validateCron(expression: string): CronValidation {
  if (expression.length > 256) return { valid: false, error: "Use a cron expression of 256 characters or fewer." };
  const fields = expression.trim().split(/\s+/);
  if (fields.length !== 5)
    return { valid: false, error: "Enter five fields: minute, hour, day of month, month, day of week." };
  const values: number[][] = [];
  for (const [index, field] of fields.entries()) {
    const { name, min, max } = FIELDS[index];
    const selected = new Set<number>();
    const parts = field.split(",");
    for (const part of parts) {
      const match = /^(\*|\d{1,2}(?:-\d{1,2})?)(?:\/(\d{1,2}))?$/.exec(part);
      if (!match || (parts.length > 1 && part.includes("*"))) {
        return {
          valid: false,
          error: `${name}: use numbers, lists, ranges, * or steps such as */2 and 1-5/2. Names and Quartz syntax are not supported.`,
        };
      }
      const [startText, endText] = match[1].split("-");
      const star = startText === "*";
      const start = star ? min : Number(startText);
      const end = star ? max : endText === undefined ? start : Number(endText);
      const step = match[2] === undefined ? 1 : Number(match[2]);
      if (start < min || end > max || start > end)
        return { valid: false, error: `${name}: use values from ${min} to ${max} and ascending ranges.` };
      if (step < 1 || step > max - min + 1 || (match[2] !== undefined && !star && endText === undefined)) {
        return {
          valid: false,
          error: `${name}: use a positive step no greater than ${max - min + 1}, after * or a range.`,
        };
      }
      for (let value = start; value <= end; value += step) selected.add(index === 4 ? value % 7 : value);
    }
    values.push([...selected].sort((a, b) => a - b));
  }

  const [minutes, hours, days, months, weekdays] = values;
  // Unix cron combines the day fields with OR only when neither starts with *.
  const daysUseOr = !fields[2].startsWith("*") && !fields[4].startsWith("*");
  if (!daysUseOr && !months.some((month) => days.some((day) => day <= MAX_DAYS[month - 1]))) {
    return { valid: false, error: "This schedule never runs: the selected months do not contain the selected days." };
  }

  const pad = (value: number) => String(value).padStart(2, "0");
  let time: string;
  if (fields[0] === "*" && fields[1] === "*") time = "Every minute";
  else if (/^\*\/\d+$/.test(fields[0]) && fields[1] === "*") {
    const step = Number(fields[0].slice(2));
    time = step === 1 ? "Every minute" : `Every ${step} minutes within each hour, starting at minute 00`;
  } else if (minutes.length === 1 && hours.length === 1) time = `At ${pad(hours[0])}:${pad(minutes[0])}`;
  else if (minutes.length === 1 && fields[1] === "*") time = `Every hour at minute ${pad(minutes[0])}`;
  else time = `At minutes ${minutes.map(pad).join(", ")} of hours ${hours.map(pad).join(", ")}`;

  const dayText = `day${days.length === 1 ? "" : "s"} ${days.join(", ")} of the month`;
  const weekdayText = weekdays.map((day) => WEEKDAYS[day]).join(", ");
  let calendar: string;
  if (fields[2] === "*" && fields[4] === "*") calendar = "every day";
  else if (fields[2] === "*") calendar = `on ${weekdayText}`;
  else if (fields[4] === "*") calendar = `on ${dayText}`;
  else calendar = `on ${dayText} ${daysUseOr ? "OR" : "AND"} on ${weekdayText}`;
  const monthText = fields[3] === "*" ? "" : `, in ${months.map((month) => MONTHS[month - 1]).join(", ")}`;
  return { valid: true, description: `${time}, ${calendar}${monthText}.` };
}
