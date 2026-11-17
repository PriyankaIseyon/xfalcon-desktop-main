import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useSmoothStreamingText } from "./useSmoothStreamingText";

const FRAME_MS = 1000 / 60;

let nextFrameId = 1;
let frames = new Map<number, FrameRequestCallback>();
let clock = 0;

/** Runs the pending animation frame callbacks, `ms` after the previous frame. */
function flushFrame(ms = FRAME_MS) {
  clock += ms;
  const pending = [...frames.values()];
  frames.clear();
  for (const callback of pending) callback(clock);
}

/** Flushes frames until the reveal stops asking for another, returning how many it took. */
function settle(limit = 600): number {
  let count = 0;
  while (frames.size > 0 && count < limit) {
    act(() => flushFrame());
    count += 1;
  }
  return count;
}

beforeEach(() => {
  nextFrameId = 1;
  frames = new Map();
  clock = 0;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    const id = nextFrameId;
    nextFrameId += 1;
    frames.set(id, callback);
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  vi.stubGlobal(
    "matchMedia",
    vi.fn(() => ({ matches: false })),
  );
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

/** The characters revealed by each frame while `source` is appended to by `arrive`. */
function revealSteps(frameCount: number, arrive: (frame: number) => number, key = "run-1"): number[] {
  let target = "";
  const { result, rerender } = renderHook(({ text }) => useSmoothStreamingText(text, true, key), {
    initialProps: { text: target },
  });
  const steps: number[] = [];
  let previous = 0;
  for (let frame = 0; frame < frameCount; frame += 1) {
    const added = arrive(frame);
    if (added > 0) {
      target += "x".repeat(added);
      rerender({ text: target });
    }
    act(() => flushFrame());
    steps.push(result.current.length - previous);
    previous = result.current.length;
  }
  return steps;
}

describe("useSmoothStreamingText", () => {
  it("waits one paint, then reveals a provider burst in order over several frames", () => {
    const target = "A response delivered as one uneven network burst.";
    const { result } = renderHook(() => useSmoothStreamingText(target, true, "run-1"));

    expect(result.current).toBe("");
    act(() => flushFrame());
    expect(result.current.length).toBeGreaterThan(0);
    expect(result.current.length).toBeLessThan(target.length);

    let previous = result.current.length;
    for (let index = 0; index < 90 && result.current !== target; index += 1) {
      act(() => flushFrame());
      expect(target.startsWith(result.current)).toBe(true);
      expect(result.current.length).toBeGreaterThanOrEqual(previous);
      previous = result.current.length;
    }
    expect(result.current).toBe(target);
  });

  it("reveals a large burst as a fast typewriter, not one block", () => {
    const steps = revealSteps(90, (frame) => (frame === 0 ? 1_500 : 0));

    // At 60 Hz the typewriter ceiling is about 37 characters a frame; the ramp
    // up and a backlog above it may exceed that a little, never reach block size.
    expect(Math.max(...steps)).toBeLessThan(150);
    expect(steps.reduce((sum, step) => sum + step, 0)).toBe(1_500);
  });

  it("smooths an uneven provider into a steady reveal", () => {
    // 60 characters every sixth frame is 600 characters a second, delivered in clumps.
    const steps = revealSteps(120, (frame) => (frame % 6 === 0 ? 60 : 0));
    const settled = steps.slice(30);

    expect(Math.min(...settled)).toBeGreaterThanOrEqual(1);
    expect(Math.max(...settled)).toBeLessThanOrEqual(20);
  });

  it("keeps the reveal speed independent of the display's frame rate", () => {
    const revealedAfterOneSecond = (frameMs: number) => {
      clock = 0;
      frames.clear();
      const { result, unmount } = renderHook(() => useSmoothStreamingText("x".repeat(5_000), true, "run-1"));
      for (let elapsed = 0; elapsed < 1000; elapsed += frameMs) act(() => flushFrame(frameMs));
      const length = result.current.length;
      unmount();
      return length;
    };

    const at60 = revealedAfterOneSecond(1000 / 60);
    const at120 = revealedAfterOneSecond(1000 / 120);
    const at30 = revealedAfterOneSecond(1000 / 30);
    expect(Math.abs(at120 - at60) / at60).toBeLessThan(0.1);
    expect(Math.abs(at30 - at60) / at60).toBeLessThan(0.1);
  });

  it("does not jump ahead after a stalled frame", () => {
    const { result } = renderHook(() => useSmoothStreamingText("x".repeat(1_500), true, "run-1"));
    for (let frame = 0; frame < 10; frame += 1) act(() => flushFrame());
    const before = result.current.length;

    act(() => flushFrame(2_000));

    // A two second gap is treated as one short frame. Taken at face value it
    // would show the whole remaining backlog at once.
    expect(result.current.length - before).toBeLessThan(300);
    expect(result.current.length).toBeLessThan(1_500);
  });

  it("clears a very large burst without a long queue", () => {
    const target = "A long response with Unicode 🚀.\n\n".repeat(600);
    const { result } = renderHook(() => useSmoothStreamingText(target, true, "burst"));

    const frameCount = settle();

    expect(result.current).toBe(target);
    expect(frameCount * FRAME_MS).toBeLessThan(3_000);
  });

  it.each(["append after catch-up", "replacement", "new run"])("restarts the pace for %s", (change) => {
    const original = "old ".repeat(2_000);
    const { result, rerender } = renderHook(({ target, key }) => useSmoothStreamingText(target, true, key), {
      initialProps: { target: original, key: "run-1" },
    });
    for (let frame = 0; frame < 20; frame += 1) act(() => flushFrame());
    const rampedStep = (() => {
      const before = result.current.length;
      act(() => flushFrame());
      return result.current.length - before;
    })();
    expect(rampedStep).toBeGreaterThan(20);

    const prefix = change === "append after catch-up" ? original : "";
    if (prefix) {
      settle();
      expect(result.current).toBe(original);
    }
    const target = prefix + "fresh ".repeat(300);
    rerender({ target, key: change === "new run" ? "run-2" : "run-1" });
    expect(result.current).toBe(prefix);
    act(() => flushFrame());
    expect(result.current.length).toBeGreaterThan(prefix.length);
    // A new burst ramps up from rest instead of inheriting the previous speed.
    expect(result.current.length - prefix.length).toBeLessThan(rampedStep);
    expect(target.startsWith(result.current)).toBe(true);
  });

  it("never exposes half of a surrogate pair while text keeps arriving", () => {
    let target = "";
    const { result, rerender } = renderHook(({ text, streaming }) => useSmoothStreamingText(text, streaming, "burst"), {
      initialProps: { text: target, streaming: true },
    });
    for (let frame = 0; frame < 40; frame += 1) {
      target += "Growing response 🚀. ".repeat(2);
      rerender({ text: target, streaming: true });
      act(() => flushFrame());
      expect(target.startsWith(result.current)).toBe(true);
      expect(result.current).not.toMatch(/[\uD800-\uDBFF]$/u);
    }
    expect(result.current.length).toBeGreaterThan(0);

    // The drain after the provider finishes is held to the same rule.
    rerender({ text: target, streaming: false });
    while (frames.size > 0) {
      act(() => flushFrame());
      expect(result.current).not.toMatch(/[\uD800-\uDBFF]$/u);
    }
    expect(result.current).toBe(target);
  });

  it("keeps revealing after the provider finishes, then shows the exact source", () => {
    const target = "The complete settled response. ".repeat(20);
    let renders = 0;
    const { result, rerender } = renderHook(
      ({ streaming }) => {
        renders += 1;
        return useSmoothStreamingText(target, streaming, "run-1");
      },
      { initialProps: { streaming: true } },
    );
    act(() => flushFrame());
    act(() => flushFrame());
    const atSettle = result.current.length;
    expect(atSettle).toBeLessThan(target.length);

    // The terminal event alone changes nothing on screen and costs no extra commit.
    const beforeSettle = renders;
    rerender({ streaming: false });
    expect(result.current.length).toBe(atSettle);
    expect(renders - beforeSettle).toBe(1);

    const drainFrames = settle();
    expect(result.current).toBe(target);
    expect(drainFrames * FRAME_MS).toBeLessThan(500);
    expect(frames.size).toBe(0);
  });

  it("shows text that was never streamed whole, with no reveal", () => {
    const { result } = renderHook(() => useSmoothStreamingText("A settled answer.", false, "run-1"));
    expect(result.current).toBe("A settled answer.");
    expect(frames.size).toBe(0);
  });

  it("never splices a replacement stream onto stale visible text", () => {
    const { result, rerender } = renderHook(({ target }) => useSmoothStreamingText(target, true, "run-1"), {
      initialProps: { target: "first answer" },
    });
    act(() => flushFrame());
    expect(result.current.length).toBeGreaterThan(0);

    rerender({ target: "replacement" });
    expect(result.current).toBe("");
    act(() => flushFrame());
    expect("replacement".startsWith(result.current)).toBe(true);
  });

  it("bypasses animation for reduced motion", () => {
    vi.stubGlobal(
      "matchMedia",
      vi.fn(() => ({ matches: true })),
    );
    const { result } = renderHook(() => useSmoothStreamingText("Show all of this now.", true, "run-1"));
    expect(result.current).toBe("Show all of this now.");
    expect(frames).toHaveLength(0);
  });
});
