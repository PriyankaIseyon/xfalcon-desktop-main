import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { ThumbsDown, ThumbsUp } from "lucide-react";
import { chatMessageFeedbackList, chatMessageFeedbackSet } from "../../lib/native";
import { IconButton } from "../../ui/IconButton";
import { Button } from "../../ui/Button";
import { getAuthAccountGeneration } from "../auth/authStore";

type Rating = 1 | -1;
type FeedbackState = {
  ratings: Record<string, number>;
  pending: ReadonlySet<string>;
  ready: boolean;
  vote: (messageId: string, rating: Rating) => Promise<void>;
};
const FeedbackContext = createContext<FeedbackState | null>(null);

/** Mounted per conversation so navigation discards votes and pending reads together. */
export function MessageFeedbackProvider({
  conversationId,
  children,
}: {
  conversationId: string | null;
  children: ReactNode;
}) {
  const [ratings, setRatings] = useState<Record<string, number>>({});
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const mounted = useRef(false);
  const pendingRef = useRef(new Set<string>());
  const account = useRef(getAuthAccountGeneration());
  const isCurrent = useCallback(() => mounted.current && account.current === getAuthAccountGeneration(), []);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (!conversationId) return;
    let cancelled = false;
    setReady(false);
    void chatMessageFeedbackList(conversationId)
      .then((rows) => {
        if (cancelled || !isCurrent()) return;
        setRatings(Object.fromEntries(rows.map((row) => [row.message_id, row.rating])));
        setReady(true);
        setError(null);
      })
      .catch(() => {
        if (!cancelled && isCurrent()) setError("Could not load response feedback.");
      });
    return () => {
      cancelled = true;
    };
  }, [conversationId, reload, isCurrent]);

  const vote = useCallback(
    async (messageId: string, rating: Rating) => {
      if (!conversationId || !ready || pendingRef.current.has(messageId)) return;
      pendingRef.current.add(messageId);
      setPending(new Set(pendingRef.current));
      const previous = ratings[messageId];
      const requested = previous === rating ? null : rating;
      setError(null);
      setRatings((current) => {
        const next = { ...current };
        if (requested === null) delete next[messageId];
        else next[messageId] = requested;
        return next;
      });
      try {
        const result = await chatMessageFeedbackSet(conversationId, messageId, requested);
        if (!isCurrent()) return;
        setRatings((current) => {
          const next = { ...current };
          if (result) next[messageId] = result.rating;
          else delete next[messageId];
          return next;
        });
        setError(null);
      } catch {
        if (isCurrent()) {
          setRatings((current) => {
            const next = { ...current };
            if (previous === undefined) delete next[messageId];
            else next[messageId] = previous;
            return next;
          });
          setError("Could not save response feedback. Please try again.");
        }
      } finally {
        pendingRef.current.delete(messageId);
        if (isCurrent()) setPending(new Set(pendingRef.current));
      }
    },
    [conversationId, isCurrent, ratings, ready],
  );

  return (
    <FeedbackContext.Provider value={{ ratings, pending, ready, vote }}>
      {error ? (
        <div className="chat-column-messages type-caption text-danger" role="alert">
          {error}{" "}
          {!ready ? (
            <Button size="sm" variant="ghost" onClick={() => setReload((value) => value + 1)}>
              Retry
            </Button>
          ) : null}
        </div>
      ) : null}
      {children}
    </FeedbackContext.Provider>
  );
}

export function MessageFeedback({ messageId, disabled }: { messageId: string; disabled: boolean }) {
  const feedback = useContext(FeedbackContext);
  if (!feedback) return null;
  return (
    <>
      {([1, -1] as const).map((rating) => {
        const selected = feedback.ratings[messageId] === rating;
        const Icon = rating === 1 ? ThumbsUp : ThumbsDown;
        const label =
          rating === 1
            ? selected
              ? "Undo upvote"
              : "Helpful response"
            : selected
              ? "Undo downvote"
              : "Unhelpful response";
        return (
          <IconButton
            key={rating}
            aria-label={label}
            selected={selected}
            className={selected ? (rating === 1 ? "feedback-upvote" : "feedback-downvote") : undefined}
            aria-disabled={feedback.pending.has(messageId) || undefined}
            size="sm"
            disabled={disabled || !feedback.ready}
            onClick={() => void feedback.vote(messageId, rating)}
          >
            <Icon aria-hidden size={14} className={selected ? "fill-current" : undefined} />
          </IconButton>
        );
      })}
    </>
  );
}
