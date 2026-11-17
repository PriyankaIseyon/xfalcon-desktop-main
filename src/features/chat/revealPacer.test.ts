import { describe, expect, it } from "vitest";
import { advancePacer, revealEnd, startPacer, type RevealPacer } from "./revealPacer";

const FRAME_MS = 1000 / 60;

/** Runs the pacer for `frames` frames over a source of `length` characters. */
function run(length: number, frames: number, settling = false, from: RevealPacer = startPacer()): RevealPacer[] {
  const states: RevealPacer[] = [];
  let pacer = from;
  for (let frame = 1; frame <= frames; frame += 1) {
    pacer = advancePacer(pacer, frame * FRAME_MS, length, settling);
    states.push(pacer);
  }
  return states;
}

describe("revealPacer", () => {
  it("never reveals past the source and finishes with the whole of it", () => {
    const states = run(300, 120);
    expect(Math.max(...states.map((state) => state.shown))).toBe(300);
    expect(states.at(-1)?.shown).toBe(300);
  });

  it("ramps the speed up instead of jumping to it", () => {
    const [first, second, third] = run(2_000, 3);
    expect(first.speed).toBeLessThan(second.speed);
    expect(second.speed).toBeLessThan(third.speed);
    // The first frame moves only a few characters even though the backlog is large.
    expect(first.shown).toBeLessThan(20);
  });

  it("reveals more per second as the backlog grows", () => {
    const small = run(60, 20).at(-1);
    const large = run(3_000, 20).at(-1);
    expect(large?.speed).toBeGreaterThan(small?.speed ?? Infinity);
  });

  it("drains faster once the provider has finished", () => {
    const streaming = run(400, 12).at(-1);
    const settling = run(400, 12, true).at(-1);
    expect(settling?.shown).toBeGreaterThan(streaming?.shown ?? Infinity);
  });

  it("rests at the floor speed once caught up, so the next burst has no momentum", () => {
    const caughtUp = run(500, 240).at(-1);
    expect(caughtUp?.shown).toBe(500);
    const rested = advancePacer(caughtUp as RevealPacer, 1_000_000, 500);
    expect(rested.speed).toBe(startPacer().speed);
  });

  it("treats a stalled frame as one short frame", () => {
    const warm = run(1_500, 10).at(-1) as RevealPacer;
    const stalled = advancePacer(warm, (warm.lastTime ?? 0) + 5_000, 1_500);
    expect(stalled.shown - warm.shown).toBeLessThan(300);
    const regular = advancePacer(warm, (warm.lastTime ?? 0) + FRAME_MS, 1_500);
    expect(stalled.shown).toBeGreaterThan(regular.shown);
  });

  it("does not run backwards when frame timestamps repeat", () => {
    const warm = run(500, 5).at(-1) as RevealPacer;
    const repeated = advancePacer(warm, warm.lastTime ?? 0, 500);
    expect(repeated.shown).toBeGreaterThanOrEqual(warm.shown);
  });
});

describe("revealEnd", () => {
  it("floors to whole characters and clamps to the source", () => {
    expect(revealEnd("hello", 2.9)).toBe(2);
    expect(revealEnd("hello", 99)).toBe(5);
    expect(revealEnd("hello", 0)).toBe(0);
  });

  it("never ends between the halves of a surrogate pair", () => {
    const text = "ab🚀cd";
    expect(revealEnd(text, 3)).toBe(4);
    expect(revealEnd(text, 4)).toBe(4);
    expect(revealEnd(text, 2)).toBe(2);
  });
});
