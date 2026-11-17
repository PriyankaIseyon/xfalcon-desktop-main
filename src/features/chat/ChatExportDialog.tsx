import { useMemo } from "react";
import { ExportDocumentDialog } from "../export/ExportDocumentDialog";
import { type ChatMessageRow, useChatSelector } from "./chatStore";
import { useTranscript } from "./useTranscript";

/**
 * The export dialog, bound to the transcript it exports.
 *
 * Mounted only while it is open, and that is the point: it reads reasoning and
 * the in-flight answer, both of which change on every token. Held mounted, it
 * would subscribe the shell around it to per-token state for a dialog nobody
 * opened (spec: chat-streaming-performance).
 *
 * Unmounting loses nothing — `ExportDocumentDialog` already resets its selection
 * on the open transition, so a fresh mount and a re-open behave identically.
 */
export function ChatExportDialog({ onClose }: { onClose: () => void }) {
  const title = useChatSelector((snapshot) => snapshot.title);
  const transcript = useTranscript();

  // The reader exports what is on screen, and while a run streams that includes
  // the prose still arriving and the prose of rounds that have not become
  // durable yet. The transcript keeps both out of `messages` so a token cannot
  // rebuild the durable array, so they are appended here instead — in the same
  // order the live timeline draws them (spec: chat-run-activity).
  const turns = useMemo(() => {
    const live: ChatMessageRow[] = [];
    transcript.roundProse.forEach((segment, index) => {
      live.push({
        id: `round-${index}`,
        role: "assistant",
        text: segment.text,
        sequence: BigInt(transcript.messages.length + live.length + 1),
        toolCalls: [],
      });
    });
    if (transcript.liveText) {
      live.push({
        id: `stream-${transcript.streamRunId ?? "active"}`,
        role: "assistant",
        text: transcript.liveText,
        sequence: BigInt(transcript.messages.length + live.length + 1),
        toolCalls: [],
      });
    }
    if (live.length === 0) return transcript.messages;
    return [...transcript.messages, ...live];
  }, [transcript.messages, transcript.roundProse, transcript.streamRunId, transcript.liveText]);

  const reasoning = useMemo(() => transcript.reasoning.map((segment) => segment.text), [transcript.reasoning]);

  return <ExportDocumentDialog open reasoning={reasoning} title={title} turns={turns} onClose={onClose} />;
}
