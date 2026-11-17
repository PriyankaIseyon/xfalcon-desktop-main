import React from "react";
import { ExternalLink } from "lucide-react";
import { cn } from "../lib/cn";

/**
 * The schemes that leave the app, and the reason the list is exactly these
 * four: `tauri_plugin_opener` injects a document-wide click handler that hands
 * `http:`, `https:`, `mailto:`, and `tel:` to the operating system — but ONLY
 * for an anchor whose target is `_blank`. Anything else stays a navigation,
 * and the navigation boundary refuses every origin but the app's own, so the
 * click does nothing at all and reports nothing.
 *
 * So `target="_blank"` here is not a window preference. It is what makes an
 * external link work, and a link outside this list must not claim to be one.
 */
const EXTERNAL_SCHEMES = ["http:", "https:", "mailto:", "tel:"] as const;

/** What the reader is about to leave for, said plainly rather than "opens in a
 * new window" — no window opens; the operating system takes over. */
const DESTINATION: Record<(typeof EXTERNAL_SCHEMES)[number], string> = {
  "http:": "opens in the browser",
  "https:": "opens in the browser",
  "mailto:": "opens the mail app",
  "tel:": "opens the phone app",
};

function externalScheme(href: string | undefined): (typeof EXTERNAL_SCHEMES)[number] | null {
  if (typeof href !== "string") return null;
  return EXTERNAL_SCHEMES.find((scheme) => href.toLowerCase().startsWith(scheme)) ?? null;
}

export interface LinkProps extends React.AnchorHTMLAttributes<HTMLAnchorElement> {
  /** Muted inline link for body copy; default is accent-forward. `nav` is the
   * unadorned row link a breadcrumb or a navigation trail wears. */
  variant?: "default" | "muted" | "subtle" | "nav";
  /** Explicit external link flag. If omitted, a leaving scheme makes it external. */
  external?: boolean;
  /** Whether to render the external link icon when external is true. */
  showExternalIcon?: boolean;
}

/**
 * The one anchor in the product.
 *
 * # Visited state
 *
 * There is deliberately none. Tailwind's preflight resets `a` to the inherited
 * colour, so nothing in this app styles `:visited`, and nothing should: a
 * desktop client renders links from model output and from remote content, and
 * a visited colour is a side channel that reports a person's browsing history
 * back to whoever authored the page. One colour for every link, before and
 * after. `check-style-contract.mjs` fails a `:visited` selector in the built
 * sheet so the policy cannot be reintroduced by a stylesheet nobody reads.
 *
 * # Colour
 *
 * `text-accent-hover` is the RESTING colour on purpose. `--xf-accent` is the
 * bright identity cyan, which does not carry body text at AA on the light
 * canvas; `--xf-accent-hover` is the same hue at a weight that does, in both
 * themes.
 */
export function Link({
  variant = "default",
  external,
  showExternalIcon = false,
  href,
  target,
  rel,
  className,
  children,
  ...props
}: LinkProps) {
  const scheme = externalScheme(href);
  const isExternal = external ?? scheme !== null;

  const resolvedTarget = target ?? (isExternal ? "_blank" : undefined);
  const resolvedRel = rel ?? (isExternal ? "noopener noreferrer" : undefined);

  return (
    <a
      href={href}
      target={resolvedTarget}
      rel={resolvedRel}
      className={cn(
        "inline-flex items-center gap-1 rounded-sm underline-offset-[3px] transition-colors",
        variant === "default" && "text-accent-hover hover:text-foreground hover:underline",
        variant === "muted" && "text-secondary hover:text-foreground hover:underline",
        variant === "subtle" && "text-faint hover:text-foreground hover:underline",
        variant === "nav" && "text-muted hover:text-foreground",
        className,
      )}
      {...props}
    >
      {children}
      {isExternal && showExternalIcon ? <ExternalLink aria-hidden size={12} className="shrink-0 opacity-70" /> : null}
      {/* Part of the accessible name, not a tooltip: a reader who cannot see
          the icon still has to be told the click leaves the app BEFORE they
          make it. Suppressed when the caller names the link itself, which
          would otherwise be announced twice over.

          The separator is inside the span and is a comma, not a space: the
          accessible-name algorithm trims each node's contribution, so a
          leading space is dropped and the phrase runs into the label. */}
      {isExternal && scheme !== null && props["aria-label"] === undefined ? (
        <span className="sr-only">{`, ${DESTINATION[scheme]}`}</span>
      ) : null}
    </a>
  );
}
