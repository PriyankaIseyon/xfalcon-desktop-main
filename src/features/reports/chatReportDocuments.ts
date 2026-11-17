import type { ReportDocument } from "../../lib/generated/contracts";

/** How many shown reports stay known. Each is an id into the local preview
 * store, not the page itself. */
const MAX_DOCUMENTS = 16;

/** The report each chat view call showed, by call id, oldest first. A long chat
 * scrolls its turns out of the list and back, and each return would otherwise
 * ask xFalcon to open the report again and reload its frame. A call is a moment
 * in the chat, so it keeps showing the report it opened; the Reports page
 * always opens the current one. */
const documents = new Map<string, Promise<ReportDocument>>();

export function chatReportDocument(callId: string, open: () => Promise<ReportDocument>): Promise<ReportDocument> {
  const held = documents.get(callId);
  if (held !== undefined) {
    documents.delete(callId);
    documents.set(callId, held);
    return held;
  }
  const pending = open();
  documents.set(callId, pending);
  // A failure is not kept: the next time the turn is shown, it asks again.
  pending.catch(() => {
    if (documents.get(callId) === pending) documents.delete(callId);
  });
  while (documents.size > MAX_DOCUMENTS) {
    const oldest = documents.keys().next().value;
    if (oldest === undefined) break;
    documents.delete(oldest);
  }
  return pending;
}

/** Forgets every shown report, on sign-out. */
export function clearChatReportDocuments() {
  documents.clear();
}
