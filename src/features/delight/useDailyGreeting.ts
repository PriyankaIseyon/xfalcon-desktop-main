import { useEffect, useRef, useState } from "react";
import { claimTodaysGreeting, GREETING_MS, pickGreeting } from "./greeting";

/**
 * The line to greet with, once on the first new-chat screen of each day, and
 * `null` the rest of the time — including after `GREETING_MS`, when it leaves.
 *
 * `enabled` is false wherever the screen is not the genuine new-chat screen, so
 * a chat that is merely loading cannot use up the day's greeting.
 */
export function useDailyGreeting(enabled: boolean, viewerName: string | null | undefined): string | null {
  const [greeting, setGreeting] = useState<string | null>(null);
  // Read when the claim lands, not a dependency: a name arriving a moment
  // later must not run a second claim, which would be told "already claimed".
  const name = useRef(viewerName);
  name.current = viewerName;

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    void claimTodaysGreeting().then((claimed) => {
      if (!cancelled && claimed) setGreeting(pickGreeting(Math.random(), new Date(), name.current));
    });
    return () => {
      cancelled = true;
    };
  }, [enabled]);

  useEffect(() => {
    if (greeting === null) return;
    const leave = setTimeout(() => setGreeting(null), GREETING_MS);
    return () => clearTimeout(leave);
  }, [greeting]);

  return greeting;
}
