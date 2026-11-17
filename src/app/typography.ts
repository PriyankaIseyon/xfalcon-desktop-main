/** Boot-time font observability (spec: app-typography).
 *
 * The bundled faces either activate or the app silently falls back to the
 * platform sans — a regression users report as "wrong font" with nothing
 * diagnosable in hand. This makes the failure loggable: one structured
 * console error per missing family, checked once fonts settle.
 */

const REQUIRED_FAMILIES = ["Google Sans", "Geist Mono"] as const;

export function reportFontActivation(): void {
  if (typeof document === "undefined" || !document.fonts?.ready) return;
  void document.fonts.ready.then(() => {
    for (const family of REQUIRED_FAMILIES) {
      if (!document.fonts.check(`16px "${family}"`)) {
        console.error("font_activation_failed", { family });
      }
    }
  });
}
