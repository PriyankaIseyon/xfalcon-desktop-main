import { delightGreetingClaim } from "../../lib/native";

/** How long the greeting stays on the new-chat screen, fade in and out included. */
export const GREETING_MS = 7000;

/** Local calendar day as `YYYY-MM-DD` — the day the person is living, not UTC's. */
export function localDay(now: Date): string {
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${now.getFullYear()}-${month}-${day}`;
}

function partOfDay(now: Date): string {
  const hour = now.getHours();
  if (hour < 12) return "Good morning";
  if (hour < 17) return "Good afternoon";
  return "Good evening";
}

/** The first word of the signed-in name, or `null` when there is none. */
function firstName(viewerName: string | null | undefined): string | null {
  return viewerName?.trim().split(/\s+/)[0] || null;
}

/** The five lines a day can open with. `pick` is a number in [0, 1). */
export function pickGreeting(pick: number, now: Date, viewerName: string | null | undefined): string {
  const name = firstName(viewerName);
  const lines = [
    name ? `${partOfDay(now)}, ${name}` : partOfDay(now),
    "Ready when you are.",
    name ? `Welcome back, ${name}.` : "Welcome back.",
    "Good to see you.",
    "Let's find some answers today.",
  ];
  return lines[Math.min(lines.length - 1, Math.floor(pick * lines.length))];
}

let inFlight: Promise<boolean> | null = null;

/**
 * Whether today's greeting is still unclaimed, claiming it. Never rejects: a
 * greeting that cannot be recorded is not shown, so a broken store cannot make
 * it show on every visit.
 *
 * Callers that ask while a claim is pending share its answer. A development
 * double-mount asks twice in one tick, and the second ask would otherwise be
 * told "already claimed" and leave the screen without the greeting the first
 * one earned.
 */
export function claimTodaysGreeting(now: Date = new Date()): Promise<boolean> {
  if (inFlight === null) {
    let claim: Promise<boolean>;
    try {
      claim = delightGreetingClaim(localDay(now));
    } catch {
      claim = Promise.resolve(false);
    }
    inFlight = claim
      .catch(() => false)
      .finally(() => {
        inFlight = null;
      });
  }
  return inFlight;
}
