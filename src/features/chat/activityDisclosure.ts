/**
 * How much of a call's own account of itself the activity card draws.
 *
 * Everything below is still projected by the backend, still stored durably, and
 * still handed to the components that would render it. These flags decide only
 * what is DRAWN, so display can be restored by flipping one constant rather than
 * by rebuilding the plumbing behind it — which is why the payload panels and the
 * step summaries all remain in `ToolActivityCard.tsx` with live call sites.
 *
 * **A call's arguments, results, and step summary** are how this product does
 * its work. The card says WHICH tools ran; it does not say what was sent to
 * them, what came back, or which objects they touched. That is a
 * data-exposure question — a tool's error text routinely quotes the rows it was
 * given — and it is decided here rather than by the reader.
 *
 * A row still shows its state, so a failed call is visibly failed and the card's
 * own label still says the run failed. Only the reason is withheld.
 *
 * The model's own reasoning used to be withheld by a third flag here. It is a
 * preference now (`appearance.show_reasoning`), because it is a verbosity
 * question rather than a data-exposure one: the reader is deciding how much of
 * the model's deliberation they want to see, and that is theirs to decide. See
 * `useShowReasoning`.
 */

/** A call's Parameters/Result panels, and a failed call's error text. */
export const SHOW_TOOL_PAYLOADS = false;

/** A call's step summary — its human label and the objects it touched. Separate
 * from the payload flag so the summary can come back on its own: it is a
 * one-line description rather than a payload, and it is the row's only
 * non-identifier content. */
export const SHOW_TOOL_STEP_SUMMARY = false;
