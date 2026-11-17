import { Skeleton } from "../../ui/Skeleton";

/** A cache-miss placeholder. The real sidebar and titlebar stay mounted. */
export function ConversationSkeleton() {
  return (
    <div
      aria-busy="true"
      aria-live="polite"
      aria-label="Loading conversation"
      className="conversation-skeleton"
      role="status"
    >
      <span className="sr-only">Loading conversation…</span>
      <div aria-hidden className="conversation-skeleton__messages">
        <div className="conversation-skeleton__turn conversation-skeleton__turn--user">
          <Skeleton className="conversation-skeleton__line conversation-skeleton__line--user" />
        </div>
        <div className="conversation-skeleton__turn">
          <Skeleton className="conversation-skeleton__line conversation-skeleton__line--wide" />
          <Skeleton className="conversation-skeleton__line" />
          <Skeleton className="conversation-skeleton__line conversation-skeleton__line--short" />
        </div>
      </div>
      <Skeleton className="conversation-skeleton__composer" />
    </div>
  );
}
