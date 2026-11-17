import { describe, expect, it, vi } from "vitest";
import { scrollChildIntoContainer } from "./scrollInContainer";

function box(top: number, height: number, scrollTop = 0): HTMLElement {
  const element = document.createElement("div");
  Object.defineProperty(element, "scrollTop", {
    configurable: true,
    get: () => scrollTop,
    set: (value: number) => {
      scrollTop = value;
    },
  });
  element.getBoundingClientRect = () =>
    ({
      top,
      bottom: top + height,
      height,
      left: 0,
      right: 100,
      width: 100,
      x: 0,
      y: top,
      toJSON: () => ({}),
    }) as DOMRect;
  return element;
}

describe("scrollChildIntoContainer", () => {
  it("moves only the given scroller so the child sits below the padding", () => {
    const container = box(0, 400, 0);
    const child = box(800, 80);
    const intoView = vi.fn();
    child.scrollIntoView = intoView;

    scrollChildIntoContainer(container, child, 20);

    expect(container.scrollTop).toBe(780);
    expect(intoView).not.toHaveBeenCalled();
  });

  it("does not jump when the child is already fully visible", () => {
    const container = box(0, 400, 40);
    const child = box(80, 60);

    scrollChildIntoContainer(container, child, 20);

    expect(container.scrollTop).toBe(40);
  });
});
