import { describe, expect, it } from "vitest";
import { isBoundRunPending, isRunPending } from "./runState";

describe("isRunPending", () => {
  it("counts every state in which the run is still working", () => {
    for (const state of ["draft", "queued", "running", "awaiting_approval", "recovering"]) {
      expect(isRunPending(state), state).toBe(true);
    }
  });

  it("treats a run parked on a tool approval as working", () => {
    // The composer must stay in its stop affordance and the live activity
    // card must keep its spinner while the prompt is on screen.
    expect(isRunPending("awaiting_approval")).toBe(true);
  });

  it("counts settled states — including the resumable ones — as done", () => {
    // `interrupted` is in flight to the Rust journal (it stays resumable) but
    // settled to the UI: the recovery notice takes over and the composer
    // comes back.
    for (const state of ["completed", "failed", "cancelled", "interrupted"]) {
      expect(isRunPending(state), state).toBe(false);
    }
  });

  it("treats no run at all as not pending", () => {
    expect(isRunPending(null)).toBe(false);
  });

  it("keeps an identified run pending until its first lifecycle event arrives", () => {
    expect(isBoundRunPending("run-1", null)).toBe(true);
    expect(isBoundRunPending(null, null)).toBe(false);
    expect(isBoundRunPending("run-1", "running")).toBe(true);
    expect(isBoundRunPending("run-1", "completed")).toBe(false);
  });
});
