import { Folder } from "lucide-react";
import { TruncatedText } from "../../ui/TruncatedText";

/** Names the project a report was created in, as a quiet chip. Used on a card, in
 * a table row and in the viewer's header, so the three say it the same way. */
export function ReportProjectChip({ name }: { name: string }) {
  return (
    <span className="report-project-chip">
      <Folder aria-hidden size={11} />
      <TruncatedText text={name}>{name}</TruncatedText>
    </span>
  );
}
