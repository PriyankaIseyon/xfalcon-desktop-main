import type { ReactNode } from "react";
import { MessageSquarePlus, Sparkles } from "lucide-react";
import { HeroTitle } from "../../ui/Heading";
import { XFalconMark } from "../../ui/XFalconMark";
import { GREETING_MS } from "../delight/greeting";
import { useDailyGreeting } from "../delight/useDailyGreeting";

/**
 * Kept in lockstep with xfalcon-web's `EmptyChatStarters` -- the two surfaces
 * must greet a signed-in user identically. Deliberately hardcoded (not derived
 * from the agent / skills / tool catalog): the prompts are workspace-agnostic
 * and the empty state must render identically regardless of what's connected.
 */
const STARTER_PROMPTS = [
  "Summarize what I can do in this workspace",
  "Walk me through the connected data sources",
  "Show what tools I have access to right now",
  "Suggest three questions worth asking my data",
] as const;

export type EmptyConversationProps = {
  canSuggest: boolean;
  onSuggest?: (prompt: string) => void;
  /** Signed-in display name for the personalized greeting, or `null` when
   * the identity is not available. */
  viewerName?: string | null;
  /** The composer, rendered centered between the greeting and the starter
   * chips -- the web welcome layout. */
  composer?: ReactNode;
  /** Opens the settings view (the desktop stand-in for web's right panel). */
  onOpenSettings?: () => void;
  /** This is the genuine new-chat screen, which may open the day with a
   * greeting. Off for any screen that only looks empty while it loads. */
  greet?: boolean;
};

export function EmptyConversation({
  canSuggest,
  onSuggest,
  viewerName,
  composer,
  onOpenSettings,
  greet = false,
}: EmptyConversationProps) {
  const greeting = useDailyGreeting(greet, viewerName);
  const trimmedName = viewerName?.trim();
  const heading = trimmedName ? `What can I help you with, ${trimmedName}?` : "What can I help you with?";

  return (
    <div className="empty-conversation flex min-h-0 flex-1 flex-col items-center overflow-y-auto py-8">
      <div className="animate-view my-auto flex w-full max-w-2xl min-w-0 flex-col items-center gap-6 text-center">
        <div className="relative flex items-center justify-center">
          <div className="absolute -inset-2 rounded-full bg-accent/10 blur-md" aria-hidden="true" />
          <XFalconMark className="empty-conversation__mark relative h-12 w-12 transition-transform duration-base hover:scale-105" />
        </div>
        {greeting ? (
          // Grows in, holds and fades out on one CSS animation of `GREETING_MS`;
          // the hook removes it when that ends.
          <div className="daily-greeting" style={{ ["--daily-greeting-ms" as string]: `${GREETING_MS}ms` }}>
            <p className="type-chrome font-medium text-secondary" role="status">
              {greeting}
            </p>
          </div>
        ) : null}
        <HeroTitle className="max-w-full break-words text-balance">{heading}</HeroTitle>
        {composer ? <div className="mt-1 w-full">{composer}</div> : null}
        <ul aria-label="Suggested prompts" className="grid w-full list-none grid-cols-1 gap-2.5 p-0 sm:grid-cols-2">
          {STARTER_PROMPTS.map((prompt) => (
            <li className="min-w-0" key={prompt}>
              <button
                className="press group flex h-full w-full cursor-pointer items-center gap-2.5 rounded-xl border border-default bg-card/80 px-4 py-3 text-left type-chrome text-secondary shadow-e1 backdrop-blur-xs transition-[border-color,background-color,color,box-shadow,transform] duration-fast hover:border-strong hover:bg-hover hover:text-foreground hover:shadow-e2 active:scale-[0.99] disabled:cursor-default disabled:opacity-50"
                disabled={!canSuggest}
                type="button"
                onClick={() => {
                  if (!canSuggest || !onSuggest) return;
                  onSuggest(prompt);
                }}
              >
                <div className="flex size-6 shrink-0 items-center justify-center rounded-full bg-surface text-faint transition-colors group-hover:bg-accent-muted group-hover:text-accent">
                  <MessageSquarePlus aria-hidden size={14} strokeWidth={1.8} />
                </div>
                <span className="min-w-0 flex-1 break-words font-medium">{prompt}</span>
              </button>
            </li>
          ))}
        </ul>
        {/* ui-extension: reads as a link and acts on the app, so it stays a
            `<button>` -- `Link` renders an `<a>`, and an `<a>` with no href is
            neither focusable nor announced as a link. */}
        {!canSuggest && onOpenSettings ? (
          <button
            className="type-chrome flex items-center gap-1.5 cursor-pointer border-0 bg-transparent text-faint underline-offset-4 transition-colors hover:text-accent hover:underline"
            type="button"
            onClick={onOpenSettings}
          >
            <Sparkles size={14} className="text-accent opacity-80" strokeWidth={1.8} />
            Add a provider API key in Settings to get started.
          </button>
        ) : null}
      </div>
    </div>
  );
}
