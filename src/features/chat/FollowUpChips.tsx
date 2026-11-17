import { useEffect, useRef, useState } from "react";
import { ArrowRight, Sparkles } from "lucide-react";
import { chatFollowUps } from "../../lib/native";
import { getAuthAccountGeneration } from "../auth/authStore";

/** BI-focused fallback when the optional model call has no useful next steps. */
function localSuggestions(content: string): string[] {
  const picks: string[] = [];
  const sql = /\b(select|from|where|join|group by)\b/i.test(content);
  if (/\b(error|failed|cannot|could not|unable to|no results|not found)\b/i.test(content))
    picks.push("Help me diagnose the data access or query issue");
  if (sql) picks.push("Check this query for duplicate rows and incorrect joins");
  if (/\b(etl|pipeline|ingestion|transform|mapping)\b/i.test(content))
    picks.push("Define data quality checks for this pipeline");
  if (/\|\s*[-:]+/.test(content)) picks.push("Chart this table");
  if (/\b(chart|plot|graph|axis|bar|line|series|visuali[sz])/i.test(content))
    picks.push("Break this down by another dimension");
  if (/\b(increase|decrease|grew|fell|rose|dropped|up|down|trend|% )/i.test(content))
    picks.push("What is driving this change?");
  if (/\d[\d,.]*\s*(%|k\b|m\b|bn\b)|\b\d{3,}\b/i.test(content))
    picks.push("How do these numbers compare to last period?");
  if (/\b(dashboard|report|kpi)\b/i.test(content)) picks.push("Review the metrics and filters in this report");
  picks.push(
    "Help me choose a dataset and business question to analyze",
    "Help me plan a KPI dashboard for my business",
    "Help me define data quality checks for an ETL pipeline",
  );
  return [...new Set(picks)].slice(0, 3);
}

/** Keyed by conversation and durable answer by the parent; never mounted while a turn runs. */
export function FollowUpChips({
  conversationId,
  assistantContent,
  userContent,
  onSend,
}: {
  conversationId: string;
  assistantContent: string;
  userContent: string | null;
  onSend: (prompt: string) => void;
}) {
  const [suggestions, setSuggestions] = useState<string[] | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const sending = useRef(false);

  useEffect(() => {
    let settled = false;
    const account = getAuthAccountGeneration();
    const commit = (items: string[]) => {
      if (settled || account !== getAuthAccountGeneration()) return;
      settled = true;
      setSuggestions(items.length ? items : localSuggestions(assistantContent));
    };
    const timer = window.setTimeout(() => commit([]), 7500);
    void chatFollowUps(conversationId, assistantContent.slice(0, 4000), userContent?.slice(0, 1000) ?? null)
      .then((result) =>
        commit(
          [
            ...new Set(
              result.suggestions
                .filter((item) => typeof item === "string" && item.trim())
                .map((item) => item.trim().slice(0, 90)),
            ),
          ].slice(0, 3),
        ),
      )
      .catch(() => commit([]))
      .finally(() => window.clearTimeout(timer));
    return () => {
      settled = true;
      window.clearTimeout(timer);
    };
  }, [assistantContent, conversationId, userContent]);

  if (dismissed) return null;
  return (
    <div
      role="group"
      aria-label="Suggested follow-ups"
      aria-busy={suggestions === null}
      className="mt-2 mb-2 flex flex-wrap items-center gap-2"
    >
      <span className="type-caption inline-flex items-center gap-1 text-muted">
        <Sparkles aria-hidden size={12} />
        Try next
      </span>
      {suggestions === null ? (
        <span role="status" className="follow-up-loading type-caption">
          Finding suggestions…
        </span>
      ) : (
        suggestions.map((prompt) => (
          <button
            key={prompt}
            type="button"
            className="follow-up-chip press type-caption inline-flex max-w-full items-center gap-1.5 rounded-full border bg-card px-3 py-1.5 text-secondary hover:bg-hover hover:text-foreground"
            onClick={() => {
              if (sending.current) return;
              sending.current = true;
              try {
                onSend(prompt);
              } finally {
                sending.current = false;
              }
            }}
          >
            <span className="min-w-0 break-words text-left">{prompt}</span>
            <ArrowRight aria-hidden size={12} className="shrink-0" />
          </button>
        ))
      )}
      <button
        type="button"
        aria-label="Dismiss follow-up suggestions"
        className="type-caption text-muted hover:text-foreground"
        onClick={() => setDismissed(true)}
      >
        Dismiss
      </button>
    </div>
  );
}
