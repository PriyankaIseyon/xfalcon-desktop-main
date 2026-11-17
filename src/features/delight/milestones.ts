import { delightMessageSent } from "../../lib/native";
import { toast } from "../../ui/toastStore";

/** Message counts that get a note. Each is reached once, so each shows once. */
const MILESTONES: ReadonlySet<number> = new Set([100, 500, 1000]);

/** How long the note stays before it closes itself. */
export const MILESTONE_NOTE_MS = 20_000;

export function milestoneNote(count: number): string | null {
  return MILESTONES.has(count)
    ? `${count} messages sent. You're a great xFalcon user. The falcon is quietly impressed.`
    : null;
}

/**
 * Counts a message that went out, and says so on a milestone. Fire and forget:
 * a count that cannot be kept must never disturb the send it follows.
 */
export function noteMessageSent(): void {
  try {
    void delightMessageSent()
      .then((count) => {
        const note = milestoneNote(count);
        if (note) toast.message(note, MILESTONE_NOTE_MS);
      })
      .catch(() => undefined);
  } catch {
    // Not a native shell, or the command is not there: nothing to count.
  }
}
