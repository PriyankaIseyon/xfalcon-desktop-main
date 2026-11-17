import { Binary, CalendarDays, Hash, Type, Braces } from "lucide-react";
import { Badge, type BadgeTone } from "../../ui/Badge";

/** Keep the server's exact type, with a visual family for scanning mixed schemas. */
export function ColumnTypeBadge({ type }: { type: string }) {
  const normalized = type.trim().toLowerCase();
  let family = "Other";
  let tone: BadgeTone = "neutral";
  let Icon = Braces;
  if (
    /^(?:u?int\d*|u?bigint|smallint|tinyint|integer|hugeint|float\d*|double|decimal\d*|numeric|number|real)\b/.test(
      normalized,
    )
  ) {
    family = "Number";
    tone = "accent";
    Icon = Hash;
  } else if (/^(?:date\d*|datetime|timestamp|time\d*|duration|interval)\b/.test(normalized)) {
    family = "Date / time";
    tone = "warn";
    Icon = CalendarDays;
  } else if (/^(?:string|large_string|utf8|varchar|nvarchar|char|text)\b/.test(normalized)) {
    family = "Text";
    tone = "success";
    Icon = Type;
  } else if (/^(?:bool|boolean)\b/.test(normalized)) {
    family = "Boolean";
    Icon = Binary;
  }
  return (
    <Badge tone={tone} aria-label={`${type} (${family})`} className="schema-type gap-1.5">
      <Icon aria-hidden size={12} className="shrink-0" />
      <span className="min-w-0 break-words">{type}</span>
    </Badge>
  );
}
