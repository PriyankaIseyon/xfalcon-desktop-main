import { cn } from "../../../lib/cn";
import { buttonClassName } from "../../../ui/buttonClassName";

/**
 * The one class every action in a card header wears — the frame's own, and
 * "Save to project", which is supplied from outside it. It is the toolbar
 * button every other surface uses (`Button` secondary, small), so a card's
 * actions look and behave exactly like the side panel's and the file view's.
 *
 * Exported rather than copied. `AssistantMessage` used to carry a byte-identical
 * literal with a comment promising to keep it in step, which is the same drift
 * the frame itself was built to end.
 */
export const ARTIFACT_ACTION_CLASS = cn(buttonClassName({ variant: "secondary", size: "sm" }), "rich-card__action");
