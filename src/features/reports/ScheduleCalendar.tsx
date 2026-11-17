import { useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { IconButton } from "../../ui/IconButton";

import { dateKey } from "./scheduleDates";

/** Calendar buttons retain native keyboard activation and visible focus. */
export function ScheduleCalendar({
  value,
  onChange,
  min,
}: {
  value: string;
  onChange: (value: string) => void;
  min: string;
}) {
  const [month, setMonth] = useState(() => new Date(`${value || min}T12:00:00`));
  const year = month.getFullYear();
  const index = month.getMonth();
  const first = new Date(year, index, 1).getDay();
  const days = new Date(year, index + 1, 0).getDate();
  const title = month.toLocaleDateString(undefined, { month: "long", year: "numeric" });
  return (
    <div className="schedule-calendar" role="group" aria-label="Start date calendar">
      <div className="schedule-calendar__heading">
        <IconButton
          aria-label="Previous month"
          disabled={dateKey(new Date(year, index, 0)) < min}
          onClick={() => setMonth(new Date(year, index - 1, 1))}
        >
          <ChevronLeft size={16} aria-hidden />
        </IconButton>
        <span aria-live="polite" className="font-medium">
          {title}
        </span>
        <IconButton
          aria-label="Next month"
          disabled={year >= 2099 && index === 11}
          onClick={() => setMonth(new Date(year, index + 1, 1))}
        >
          <ChevronRight size={16} aria-hidden />
        </IconButton>
      </div>
      <div className="schedule-calendar__days">
        {["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].map((day) => (
          <span key={day} className="type-micro text-muted" aria-hidden>
            {day}
          </span>
        ))}
        {Array.from({ length: first }, (_, i) => (
          <span key={`blank-${i}`} />
        ))}
        {Array.from({ length: days }, (_, i) => {
          const date = new Date(year, index, i + 1);
          const key = dateKey(date);
          return (
            <button
              type="button"
              key={key}
              aria-label={date.toLocaleDateString(undefined, { dateStyle: "full" })}
              aria-pressed={value === key}
              disabled={key < min}
              onClick={() => onChange(key)}
            >
              {i + 1}
            </button>
          );
        })}
      </div>
    </div>
  );
}
