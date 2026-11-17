import { cn } from "../../lib/cn";

/**
 * The response boundary (spec: chat-thread-layout), xfalcon-web's
 * `ResponseRibbon`: a labelled hairline above every assistant answer — the
 * one that follows a tool run and the one that is a direct reply alike — so
 * the handoff into the answer reads the same everywhere in the transcript.
 *
 * `tone` follows the row's own `isLatestAssistant`, so the rule and the
 * vertical rail beside it always say the same thing about the same answer:
 * accent on the newest, neutral on everything above it.
 *
 * Otherwise deliberately static. The in-flight stream row and the durable row
 * that replaces it paint the identical ribbon, so the settle has nothing to
 * re-animate and the boundary never flashes.
 *
 * `aria-hidden`: the assistant turn already carries its role for assistive
 * tech, and repeating "Response" before every answer is noise there.
 */
export function ResponseRibbon({ tone }: { tone: "latest" | "history" }) {
  return (
    <div aria-hidden className={cn("response-ribbon", tone === "latest" && "response-ribbon--latest")}>
      <span className="response-ribbon__label">Response</span>
      <span className="response-ribbon__rule" />
    </div>
  );
}
