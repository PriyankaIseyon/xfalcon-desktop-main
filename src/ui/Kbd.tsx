import React from "react";
import { cn } from "../lib/cn";
import { isApplePlatform, usePlatform } from "../lib/platform";

export type KbdProps = React.HTMLAttributes<HTMLElement>;

/** One key cap. `Shortcut` below is what a caller reaches for; this is the
 * piece it is made of, and the escape hatch for a lone key in prose. */
export function Kbd({ className, ...props }: KbdProps) {
  return (
    <kbd
      className={cn(
        "font-mono text-2xs px-1.5 py-0.5 rounded-sm border border-default bg-surface text-secondary",
        className,
      )}
      {...props}
    />
  );
}

/**
 * The keys this app names, spelled once.
 *
 * `Mod` is the platform's own command modifier. Naming it that way is the whole
 * point: a caller states the shortcut, not the spelling, so the same shortcut
 * cannot come out three different ways — which is exactly what it did. The
 * command palette was labelled `Ctrl K` in the sidebar, `Ctrl+K` in the rail,
 * and `⌘/Ctrl K` in the cheat sheet, and the cheat sheet showed BOTH platforms'
 * modifiers to everyone on every platform.
 */
const KEY_LABELS: Record<string, { apple: string; other: string; spoken: string }> = {
  Mod: { apple: "⌘", other: "Ctrl", spoken: "Command" },
  Shift: { apple: "⇧", other: "Shift", spoken: "Shift" },
  Alt: { apple: "⌥", other: "Alt", spoken: "Option" },
  Enter: { apple: "↩", other: "Enter", spoken: "Enter" },
  Escape: { apple: "esc", other: "Esc", spoken: "Escape" },
  Up: { apple: "↑", other: "↑", spoken: "Up arrow" },
  Down: { apple: "↓", other: "↓", spoken: "Down arrow" },
};

/** The non-Apple modifier word, which is also what `unknown` renders. */
function label(key: string, apple: boolean): string {
  const known = KEY_LABELS[key];
  if (!known) return key;
  return apple ? known.apple : known.other;
}

function spoken(key: string, apple: boolean): string {
  const known = KEY_LABELS[key];
  if (!known) return key;
  // The Apple modifier glyphs have Unicode names a screen reader will happily
  // read out — "place of interest sign" for ⌘. The word is what gets spoken on
  // every platform; only the printed label changes.
  return known.spoken === "Command" && !apple ? "Control" : known.spoken;
}

export interface ShortcutProps {
  /** Canonical keys, e.g. `["Mod", "Shift", "F"]`. `Mod` is the platform's own
   * command modifier. */
  keys: readonly string[];
  className?: string;
}

/**
 * One keyboard shortcut, printed the way the host platform prints it.
 *
 * Apple runs its modifier glyphs together with no separator (`⌘⇧F`); everything
 * else joins with a plus (`Ctrl+Shift+F`). That is a real typographic
 * difference, not a preference, so the primitive owns it and no caller spells
 * a modifier itself.
 *
 * The caps are hidden from assistive tech and the whole run carries one name,
 * because reading them individually gives either a glyph with an absurd Unicode
 * name or a string of disconnected letters. `role="img"` is the same treatment
 * `StatusDot` uses for a mark plus its text.
 */
export function Shortcut({ keys, className }: ShortcutProps) {
  const apple = isApplePlatform(usePlatform());
  const separator = apple ? "" : "+";

  return (
    <span
      aria-label={keys.map((key) => spoken(key, apple)).join(" plus ")}
      className={cn("inline-flex items-center", apple ? "gap-0.5" : "gap-1", className)}
      role="img"
    >
      {keys.map((key, index) => (
        <React.Fragment key={`${key}-${index}`}>
          {index > 0 && separator ? (
            <span aria-hidden className="text-faint">
              {separator}
            </span>
          ) : null}
          <Kbd aria-hidden>{label(key, apple)}</Kbd>
        </React.Fragment>
      ))}
    </span>
  );
}
