// @vitest-environment node

import { describe, expect, it } from "vitest";
import { MIGRATION_ACK_TARGET, runStorageMigration } from "./storage-migration";

function storage(
  name: string,
  events: string[],
  remaining: number,
  clearError?: Error,
): Pick<Storage, "clear" | "length"> {
  return {
    clear() {
      events.push(`${name}.clear`);
      if (clearError) throw clearError;
    },
    get length() {
      events.push(`${name}.length`);
      return remaining;
    },
  };
}

describe("origin storage migration", () => {
  it("clears both stores, verifies both, and only then navigates to the fixed acknowledgement", () => {
    const events: string[] = [];

    runStorageMigration(storage("local", events, 0), storage("session", events, 0), (target) =>
      events.push(`navigate:${target}`),
    );

    expect(events).toEqual([
      "local.clear",
      "session.clear",
      "local.length",
      "session.length",
      `navigate:${MIGRATION_ACK_TARGET}`,
    ]);
  });

  it("does not navigate when either store remains nonempty", () => {
    const events: string[] = [];

    expect(() =>
      runStorageMigration(storage("local", events, 1), storage("session", events, 0), (target) =>
        events.push(`navigate:${target}`),
      ),
    ).toThrow();
    expect(events).not.toContain(`navigate:${MIGRATION_ACK_TARGET}`);
  });

  it("does not navigate when clearing throws", () => {
    const events: string[] = [];

    expect(() =>
      runStorageMigration(storage("local", events, 0, new Error("blocked")), storage("session", events, 0), (target) =>
        events.push(`navigate:${target}`),
      ),
    ).toThrow();
    expect(events).toEqual(["local.clear"]);
  });
});
