/**
 * Vendored from Vercel AI Elements' shadcn registry dependency
 * (`collapsible`, Apache-2.0) — the thin Radix re-export shadcn ships,
 * minus Next.js directives. Radix owns the a11y contract
 * (`aria-expanded`/`aria-controls`, `data-state`) and exposes
 * `--radix-collapsible-content-height` for height-correct collapse
 * animation (see `.ai-collapsible-content` in `src/index.css`).
 */
import {
  CollapsibleContent as RadixCollapsibleContent,
  CollapsibleTrigger as RadixCollapsibleTrigger,
  Root as RadixCollapsibleRoot,
} from "@radix-ui/react-collapsible";

export const Collapsible = RadixCollapsibleRoot;
export const CollapsibleTrigger = RadixCollapsibleTrigger;
export const CollapsibleContent = RadixCollapsibleContent;
