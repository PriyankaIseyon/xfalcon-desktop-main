import { useContext, type ReactNode } from "react";
import {
  HighlightScopeContext,
  RevealContext,
  splitHighlighted,
  useIsSearchRevealTarget,
  type SearchReveal,
} from "./searchReveal";

/**
 * Components only. The contexts, hooks, and match-splitting live in
 * `./searchReveal` so this file exports nothing but components and Fast
 * Refresh keeps working while a search is open.
 */

export function SearchRevealProvider({ value, children }: { value: SearchReveal | null; children: ReactNode }) {
  return <RevealContext.Provider value={value}>{children}</RevealContext.Provider>;
}

/** Marks matching text only inside the element the palette jumped to. */
export function SearchHighlightScope({ active, children }: { active: boolean; children: ReactNode }) {
  return <HighlightScopeContext.Provider value={active}>{children}</HighlightScopeContext.Provider>;
}

export function SearchRevealHost({ id, children }: { id: string; children: ReactNode }) {
  const active = useIsSearchRevealTarget(id);
  return (
    <div id={id} data-search-reveal={active ? "" : undefined}>
      <SearchHighlightScope active={active}>{children}</SearchHighlightScope>
    </div>
  );
}

export function HighlightedText({ text }: { text: string }) {
  const reveal = useContext(RevealContext);
  const inScope = useContext(HighlightScopeContext);
  if (!inScope || !reveal?.query.trim()) return text;

  const parts = splitHighlighted(text, reveal.query);
  return (
    <>
      {parts.map((part, index) =>
        part.hit ? (
          <mark className="search-reveal-mark" key={index}>
            {part.value}
          </mark>
        ) : (
          part.value
        ),
      )}
    </>
  );
}
