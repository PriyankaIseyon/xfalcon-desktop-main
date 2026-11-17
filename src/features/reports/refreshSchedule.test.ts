import { describe, expect, it } from "vitest";
import { validateCron } from "./refreshSchedule";

describe("validateCron", () => {
  it.each([
    ["* * * * *", "Every minute, every day."],
    ["0 9 * * *", "At 09:00, every day."],
    ["  30  8 * * 1-5  ", "At 08:30, on Monday, Tuesday, Wednesday, Thursday, Friday."],
    ["15 * * * *", "Every hour at minute 15, every day."],
    ["*/15 * * * *", "Every 15 minutes within each hour, starting at minute 00, every day."],
    ["*/7 * * * *", "Every 7 minutes within each hour, starting at minute 00, every day."],
    ["0 0 1 * *", "At 00:00, on day 1 of the month."],
    ["0 9 * * 0,7", "At 09:00, on Sunday."],
    ["0 9 29 2 *", "At 09:00, on day 29 of the month, in February."],
    [
      "0,30 9-13/2 1,15 1,6 1-5/2",
      "At minutes 00, 30 of hours 09, 11, 13, on days 1, 15 of the month OR on Monday, Wednesday, Friday, in January, June.",
    ],
    ["0 9 30 2 1", "At 09:00, on day 30 of the month OR on Monday, in February."],
    ["0 9 1 * */2", "At 09:00, on day 1 of the month AND on Sunday, Tuesday, Thursday, Saturday."],
  ])("describes %s accurately", (expression, description) => {
    expect(validateCron(expression)).toEqual({ valid: true, description });
  });

  it.each([
    "",
    "0 9 * *",
    "0 0 9 * * *",
    "@daily",
    "60 9 * * *",
    "0 24 * * *",
    "0 9 0 * *",
    "0 9 32 * *",
    "0 9 * 0 *",
    "0 9 * 13 *",
    "0 9 * * 8",
    "0 9 * * MON",
    "0 9 ? * 1",
    "0 9 L * *",
    "0 9 * * 1#2",
    "0 9 1W * *",
    "*/0 * * * *",
    "*/-1 * * * *",
    "*/61 * * * *",
    "0 9/2 * * *",
    "5-1 * * * *",
    "0 9 * * 5-1",
    "0,,1 9 * * *",
    "0, 9 * * *",
    "*,1 9 * * *",
    "1-3/0 9 * * *",
    "1/2/3 9 * * *",
    "1.5 9 * * *",
    "1000000000000 9 * * *",
    "0 9 30 2 *",
    "0 9 31 4,6,9,11 *",
    "0 9 30 2 */2",
    "0 9 * * *; alert(1)",
    " ".repeat(257),
  ])("rejects invalid expression %s", (expression) => {
    const result = validateCron(expression);
    expect(result.valid).toBe(false);
    if (!result.valid) expect(result.error.length).toBeGreaterThan(10);
  });

  it("allows day-of-month ranges if any selected date exists", () => {
    expect(validateCron("0 9 28-31 2 *").valid).toBe(true);
  });

  it("does not treat an explicit full weekday range as a wildcard in Unix day semantics", () => {
    const result = validateCron("0 9 30 2 0-7");
    expect(result.valid).toBe(true);
    if (result.valid) expect(result.description).toContain(" OR ");
  });
});
