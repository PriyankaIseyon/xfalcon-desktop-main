import { useId, useState } from "react";
import { CalendarClock, Download, Mail } from "lucide-react";
import { Button } from "../../ui/Button";
import { Field, Input, Textarea } from "../../ui/Field";
import { Modal } from "../../ui/Modal";
import { Select } from "../../ui/Select";
import { Alert } from "../../ui/Alert";
import { toast } from "../../ui/toastStore";
import { ScheduleCalendar } from "./ScheduleCalendar";
import { dateKey, zonedNow } from "./scheduleDates";
import { validateCron } from "./refreshSchedule";
import "./reportTools.css";

type Action = "schedule" | "email";
type Frequency = "once" | "hourly" | "daily" | "weekly" | "monthly" | "custom";
const titles: Record<Action, string> = {
  schedule: "Schedule a refresh",
  email: "Share via email",
};
const weekdays = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const frequencies: { value: Frequency; label: string }[] = [
  { value: "once", label: "Once" },
  { value: "hourly", label: "Hourly" },
  { value: "daily", label: "Daily" },
  { value: "weekly", label: "Weekly" },
  { value: "monthly", label: "Monthly" },
  { value: "custom", label: "Custom cron" },
];

/**
 * The viewer's secondary actions. Export is real: it asks the backend, which opens
 * the native save dialog and writes the file the reader names. Schedule and Share
 * via email stay labeled previews, because DA has no endpoint for either.
 */
export function ReportTools({
  title,
  disabled,
  exportLabel,
  exporting,
  onExport,
}: {
  title: string;
  disabled: boolean;
  /** What exporting writes, for the button's tooltip: "HTML file", "Excel workbook". */
  exportLabel: string;
  exporting: boolean;
  onExport: () => void;
}) {
  const [action, setAction] = useState<Action | null>(null);
  return (
    <>
      <div className="report-tools" role="group" aria-label="Report actions">
        <Button variant="secondary" disabled={disabled} onClick={() => setAction("schedule")}>
          <CalendarClock size={14} aria-hidden />
          Schedule a refresh
        </Button>
        <Button variant="secondary" disabled={disabled} onClick={() => setAction("email")}>
          <Mail size={14} aria-hidden />
          Share via email
        </Button>
        <Button
          aria-label={`Export as ${exportLabel}`}
          variant="secondary"
          disabled={disabled || exporting}
          loading={exporting}
          onClick={onExport}
        >
          {exporting ? null : <Download size={14} aria-hidden />}
          Export
        </Button>
      </div>
      {action ? <ReportToolDialog key={action} action={action} title={title} onClose={() => setAction(null)} /> : null}
    </>
  );
}

function ReportToolDialog({ action, title, onClose }: { action: Action; title: string; onClose: () => void }) {
  const formId = useId();
  const [frequency, setFrequency] = useState<Frequency>("daily");
  const [start, setStart] = useState(() => dateKey(new Date(Date.now())));
  const [end, setEnd] = useState("");
  const [time, setTime] = useState("09:00");
  const [zone, setZone] = useState(() => Intl.DateTimeFormat().resolvedOptions().timeZone);
  const [days, setDays] = useState([1]);
  const [monthDay, setMonthDay] = useState("1");
  const [interval, setInterval] = useState("1");
  const [cron, setCron] = useState("0 9 * * 1-5");
  const [email, setEmail] = useState("");
  const [subject, setSubject] = useState(title.slice(0, 200));
  const [message, setMessage] = useState("");
  const [success, setSuccess] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const current = zonedNow(zone);
  const today = current?.date ?? dateKey(new Date(Date.now()));
  const currentTime = current?.time ?? "00:00";
  const zoneValid = current !== null;
  const cronResult = validateCron(cron);
  const dateValid = (value: string) =>
    /^\d{4}-\d{2}-\d{2}$/.test(value) &&
    !Number.isNaN(Date.parse(`${value}T12:00:00`)) &&
    dateKey(new Date(`${value}T12:00:00`)) === value;
  const dateError =
    !dateValid(start) || start < today || start > "2099-12-31" ? "Choose a start date between today and 2099." : null;
  const endError =
    end && (!dateValid(end) || end < start || end > "2099-12-31")
      ? "End date must be on or after the start date, through 2099."
      : null;
  const timeError =
    frequency === "once" && start === today && time <= currentTime
      ? "Choose a future time in the selected timezone."
      : !/^([01]\d|2[0-3]):[0-5]\d$/.test(time)
        ? "Enter a valid time."
        : null;
  const recipients = email
    .split(/[,;]+/)
    .map((s) => s.trim())
    .filter(Boolean);
  const emailError =
    recipients.length === 0 ||
    recipients.length > 10 ||
    recipients.some((s) => s.length > 254 || !/^[^\s@,;<>]+@[^\s@,;<>]+\.[^\s@,;<>]+$/.test(s))
      ? "Enter 1–10 valid email addresses, separated by commas."
      : null;
  const scheduleError =
    dateError ||
    (frequency !== "once" && endError) ||
    !zoneValid ||
    (frequency !== "custom" && timeError) ||
    (frequency === "weekly" && days.length === 0) ||
    (frequency === "monthly" && (!/^\d+$/.test(monthDay) || +monthDay < 1 || +monthDay > 31)) ||
    (frequency === "custom" && !cronResult.valid);
  const canConfirm = action === "schedule" ? !scheduleError : !emailError && subject.trim().length > 0;
  let scheduleText: string;
  if (frequency === "custom") scheduleText = cronResult.valid ? cronResult.description : "";
  else if (frequency === "once") scheduleText = `Once on ${start} at ${time}.`;
  else if (frequency === "hourly")
    scheduleText = `Every ${interval} hour${interval === "1" ? "" : "s"}, starting at ${time} on ${start}.`;
  else if (frequency === "daily") scheduleText = `Every day at ${time}.`;
  else if (frequency === "weekly")
    scheduleText = `Every ${days
      .slice()
      .sort()
      .map((d) => weekdays[d])
      .join(", ")} at ${time}.`;
  else scheduleText = `On day ${monthDay} of each month at ${time}. Months without this date are skipped.`;
  const successText = {
    schedule: "Demo: schedule confirmed. No refresh was scheduled.",
    email: "Demo: email confirmed. No email was sent.",
  }[action];
  return (
    <Modal
      open
      title={titles[action]}
      description={title}
      onClose={onClose}
      size={action === "schedule" ? "lg" : "md"}
      footer={
        success ? (
          <Button variant="primary" onClick={onClose}>
            Done
          </Button>
        ) : (
          <>
            <Button variant="secondary" onClick={onClose}>
              Cancel
            </Button>
            <Button variant="primary" type="submit" form={formId} disabled={!canConfirm}>
              Confirm {action === "email" ? "email" : action}
            </Button>
          </>
        )
      }
    >
      {success ? (
        <Alert tone="success" title="Preview complete">
          {successText}
        </Alert>
      ) : (
        <form
          id={formId}
          className="report-tool-form"
          onChange={() => setSubmitError(null)}
          onSubmit={(e) => {
            e.preventDefault();
            if (!canConfirm) return;
            if (action === "schedule") {
              const fresh = zonedNow(zone);
              if (
                !fresh ||
                start < fresh.date ||
                (frequency === "once" && start === fresh.date && time <= fresh.time)
              ) {
                setSubmitError("The start time has passed. Choose a future date and time.");
                return;
              }
            }
            setSuccess(true);
            toast.success(successText);
          }}
        >
          <p className="type-caption text-muted">Demo mode · No changes are applied.</p>
          {submitError ? (
            <p role="alert" className="type-caption text-danger">
              {submitError}
            </p>
          ) : null}
          {action === "schedule" ? (
            <>
              <Field label="Frequency">
                <Select value={frequency} onChange={setFrequency} options={frequencies} />
              </Field>
              <div className="schedule-layout">
                <div className="report-tool-form">
                  <Field label="Start date" error={dateError}>
                    <Input
                      type="date"
                      value={start}
                      min={today}
                      max="2099-12-31"
                      onChange={(e) => setStart(e.target.value)}
                    />
                  </Field>
                  <ScheduleCalendar
                    key={start.slice(0, 7)}
                    value={dateValid(start) ? start : today}
                    min={today}
                    onChange={setStart}
                  />
                </div>
                <div className="report-tool-form">
                  <Field
                    label="Timezone"
                    error={!zoneValid ? "Enter a valid IANA timezone, such as Asia/Kolkata." : undefined}
                  >
                    <Input
                      list={`${formId}-zones`}
                      value={zone}
                      maxLength={80}
                      onChange={(e) => setZone(e.target.value)}
                    />
                  </Field>
                  <datalist id={`${formId}-zones`}>
                    {Array.from(
                      new Set([
                        Intl.DateTimeFormat().resolvedOptions().timeZone,
                        "UTC",
                        "Asia/Kolkata",
                        "Asia/Singapore",
                        "Europe/London",
                        "America/New_York",
                        "America/Los_Angeles",
                        "Australia/Sydney",
                      ]),
                    ).map((z) => (
                      <option key={z} value={z} />
                    ))}
                  </datalist>
                  {frequency !== "custom" ? (
                    <Field label={frequency === "hourly" ? "First refresh time" : "Time"} error={timeError}>
                      <Input type="time" value={time} onChange={(e) => setTime(e.target.value)} />
                    </Field>
                  ) : null}
                  {frequency === "hourly" ? (
                    <Field label="Repeat every">
                      <Select
                        value={interval}
                        onChange={setInterval}
                        options={[1, 2, 4, 6, 8, 12].map((n) => ({
                          value: String(n),
                          label: `${n} hour${n === 1 ? "" : "s"}`,
                        }))}
                      />
                    </Field>
                  ) : null}
                  {frequency === "monthly" ? (
                    <Field label="Day of month" hint="Months without this date are skipped.">
                      <Input
                        type="number"
                        min={1}
                        max={31}
                        value={monthDay}
                        onChange={(e) => setMonthDay(e.target.value)}
                      />
                    </Field>
                  ) : null}
                  {frequency !== "once" ? (
                    <Field label="End date" hint="Optional · Leave blank to continue indefinitely." error={endError}>
                      <Input
                        type="date"
                        min={start || today}
                        max="2099-12-31"
                        value={end}
                        onChange={(e) => setEnd(e.target.value)}
                      />
                    </Field>
                  ) : null}
                </div>
              </div>
              {frequency === "weekly" ? (
                <fieldset>
                  <legend className="type-caption mb-2">Repeat on</legend>
                  <div className="schedule-weekdays">
                    {weekdays.map((day, index) => (
                      <button
                        type="button"
                        key={day}
                        aria-label={day}
                        aria-pressed={days.includes(index)}
                        onClick={() =>
                          setDays((old) => (old.includes(index) ? old.filter((d) => d !== index) : [...old, index]))
                        }
                      >
                        {day.slice(0, 3)}
                      </button>
                    ))}
                  </div>
                  {days.length === 0 ? (
                    <p role="alert" className="type-caption text-danger">
                      Select at least one day.
                    </p>
                  ) : null}
                </fieldset>
              ) : null}
              {frequency === "custom" ? (
                <Field
                  label="Cron expression"
                  hint="Five numeric fields: minute, hour, day of month, month, weekday (0 or 7 = Sunday). Supports *, lists, ranges and steps."
                  error={!cronResult.valid ? cronResult.error : undefined}
                >
                  <Input value={cron} maxLength={200} spellCheck={false} onChange={(e) => setCron(e.target.value)} />
                </Field>
              ) : null}
              {scheduleText ? (
                <div className="schedule-summary" role="status">
                  <p className="font-medium">{scheduleText}</p>
                  <p className="type-caption text-muted">
                    {zone}
                    {frequency !== "once" ? ` · From ${start}${end ? ` through ${end}` : " · No end date"}` : ""}
                  </p>
                  <p className="type-caption text-muted">
                    Times follow the selected timezone, including daylight saving changes.
                  </p>
                </div>
              ) : null}
            </>
          ) : null}
          {action === "email" ? (
            <>
              <Field
                label="Recipients"
                hint="Up to 10 addresses, separated by commas."
                error={email && emailError ? emailError : undefined}
              >
                <Input
                  value={email}
                  maxLength={2550}
                  placeholder="name@example.com"
                  onChange={(e) => setEmail(e.target.value)}
                />
              </Field>
              <Field label="Subject" required>
                <Input value={subject} maxLength={200} onChange={(e) => setSubject(e.target.value)} />
              </Field>
              <Field label="Message" hint="Optional">
                <Textarea rows={3} maxLength={2000} value={message} onChange={(e) => setMessage(e.target.value)} />
              </Field>
              <p className="type-caption text-muted">
                Email includes a report link. Recipients must have access to the workspace.
              </p>
            </>
          ) : null}
        </form>
      )}
    </Modal>
  );
}
