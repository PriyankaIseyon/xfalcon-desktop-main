import React from "react";
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SegmentedControl } from "./SegmentedControl";
import { Tabs } from "./Tabs";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function rect(left: number, top = 20, width = 64, height = 32): DOMRect {
  return {
    bottom: top + height,
    height,
    left,
    right: left + width,
    top,
    width,
    x: left,
    y: top,
    toJSON: () => ({}),
  } as DOMRect;
}

function installStripGeometry() {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    if (this.matches('[role="radiogroup"], [role="tablist"]')) return rect(100, 20, 260, 40);
    if (this.matches('[role="radio"], [role="tab"]')) {
      const buttons = Array.from(
        this.parentElement?.querySelectorAll<HTMLElement>('[role="radio"], [role="tab"]') ?? [],
      );
      const index = buttons.indexOf(this);
      return rect(108 + index * 80, 24, 48 + index * 12, 32);
    }
    return rect(0, 0, 0, 0);
  });
  for (const [property, field, origin] of [
    ["offsetLeft", "left", 100],
    ["offsetTop", "top", 20],
    ["offsetWidth", "width", 0],
    ["offsetHeight", "height", 0],
  ] as const) {
    vi.spyOn(HTMLElement.prototype, property, "get").mockImplementation(function (this: HTMLElement) {
      return this.getBoundingClientRect()[field] - origin;
    });
  }
}

describe("shared elastic selection", () => {
  it("drags to the nearest enabled option and keeps an immediate click operable", () => {
    function Harness() {
      const [value, setValue] = React.useState("a");
      return (
        <SegmentedControl
          aria-label="Display mode"
          onChange={setValue}
          options={[
            { label: "Alpha", value: "a" },
            { label: "Unavailable", value: "b", disabled: true },
            { label: "Charlie", value: "c" },
          ]}
          value={value}
        />
      );
    }

    installStripGeometry();
    render(<Harness />);
    const group = screen.getByRole("radiogroup", { name: "Display mode" });
    const buttons = within(group).getAllByRole("radio");
    Object.defineProperty(group, "scrollWidth", { configurable: true, value: 260 });
    const thumb = group.querySelector<HTMLElement>("[data-elastic-thumb]");
    expect(thumb).toHaveAttribute("aria-hidden", "true");
    expect(thumb?.style.left).toBe("8px");

    fireEvent.pointerDown(buttons[0]!, { button: 0, clientX: 130, pointerId: 7 });
    fireEvent.pointerMove(group, { clientX: 276, pointerId: 7 });
    fireEvent.pointerUp(group, { clientX: 276, pointerId: 7 });
    expect(thumb?.style.left).toBe("168px");
    // A fresh activation after release must be handled, even if it happens
    // before the drag reconciliation frame runs.
    fireEvent.click(buttons[0]!);

    expect(buttons[0]).toHaveAttribute("aria-checked", "true");
    expect(buttons[1]).toBeDisabled();
    expect(thumb?.style.left).toBe("8px");
  });

  it("settles a rejected controlled drag back to the parent's selected value", () => {
    const onChange = vi.fn();
    installStripGeometry();
    render(
      <SegmentedControl
        aria-label="Display mode"
        onChange={onChange}
        options={[
          { label: "Alpha", value: "a" },
          { label: "Bravo", value: "b" },
        ]}
        value="a"
      />,
    );

    const group = screen.getByRole("radiogroup", { name: "Display mode" });
    const buttons = within(group).getAllByRole("radio");
    Object.defineProperty(group, "scrollWidth", { configurable: true, value: 260 });
    const thumb = group.querySelector<HTMLElement>("[data-elastic-thumb]");
    let reconcile: FrameRequestCallback | undefined;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      reconcile = callback;
      return 23;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {
      reconcile = undefined;
    });
    fireEvent.pointerDown(buttons[0]!, { button: 0, clientX: 130, pointerId: 3 });
    fireEvent.pointerMove(group, { clientX: 190, pointerId: 3 });
    fireEvent.pointerUp(group, { clientX: 190, pointerId: 3 });
    expect(thumb?.style.left).not.toBe("8px");
    act(() => {
      reconcile?.(0);
    });

    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange).toHaveBeenCalledWith("b");
    expect(buttons[0]).toHaveAttribute("aria-checked", "true");
    expect(thumb?.style.left).toBe("8px");
  });

  it("cancels the thumb animation and hides it when every option becomes disabled", () => {
    function Harness({ disabled, value }: { disabled: boolean; value: string }) {
      return (
        <SegmentedControl
          aria-label="Display mode"
          onChange={() => {}}
          options={[
            { label: "Alpha", value: "a", disabled },
            { label: "Bravo", value: "b", disabled },
          ]}
          value={value}
        />
      );
    }

    installStripGeometry();
    const view = render(<Harness disabled={false} value="a" />);
    const group = screen.getByRole("radiogroup", { name: "Display mode" });
    const thumb = group.querySelector<HTMLElement>("[data-elastic-thumb]")!;
    const cancel = vi.fn();
    const animate = vi.fn<(frames: Keyframe[], options?: KeyframeAnimationOptions) => Animation>(
      () => ({ cancel }) as unknown as Animation,
    );
    Object.defineProperty(thumb, "animate", {
      configurable: true,
      value: animate,
    });
    view.rerender(<Harness disabled={false} value="b" />);
    expect(animate).toHaveBeenCalledOnce();
    const frames = animate.mock.calls[0]?.[0] ?? [];
    expect(frames.every((frame) => "transform" in frame && !("left" in frame))).toBe(true);
    expect(String(frames[0]?.transform)).toContain("scale(0.8, 1)");

    view.rerender(<Harness disabled value="a" />);
    expect(cancel).toHaveBeenCalledOnce();
    expect(thumb.style.opacity).toBe("0");
  });

  it("uses the same decorative measured thumb for tabs and preserves the single panel", () => {
    installStripGeometry();
    render(
      <Tabs
        aria-label="File sections"
        onChange={() => {}}
        tabs={[
          { id: "file", label: "File" },
          { id: "history", label: "History" },
        ]}
        value="file"
      >
        File body
      </Tabs>,
    );

    const tablist = screen.getByRole("tablist", { name: "File sections" });
    const tabs = within(tablist).getAllByRole("tab");
    const thumb = tablist.querySelector<HTMLElement>("[data-elastic-thumb]");
    expect(thumb).toHaveAttribute("aria-hidden", "true");
    expect(thumb).toHaveClass("elastic-selection__thumb");
    expect(tabs[0]).toHaveAttribute("aria-controls", screen.getByRole("tabpanel").id);
    expect(tabs[1]).not.toHaveAttribute("aria-controls");
  });

  it("keeps an in-flight transition during another press and cancels it on unmount", () => {
    installStripGeometry();
    const options = [
      { label: "Alpha", value: "a" },
      { label: "Bravo", value: "b" },
    ];
    const view = render(<SegmentedControl aria-label="Mode" options={options} value="a" onChange={() => {}} />);
    const group = screen.getByRole("radiogroup", { name: "Mode" });
    const thumb = group.querySelector<HTMLElement>("[data-elastic-thumb]")!;
    const cancel = vi.fn();
    Object.defineProperty(thumb, "animate", { configurable: true, value: () => ({ cancel }) });
    view.rerender(<SegmentedControl aria-label="Mode" options={options} value="b" onChange={() => {}} />);
    fireEvent.pointerDown(screen.getByRole("radio", { name: "Bravo" }), { button: 0, clientX: 205, pointerId: 5 });
    fireEvent.pointerUp(group, { clientX: 205, pointerId: 5 });
    expect(cancel).not.toHaveBeenCalled();
    view.unmount();
    expect(cancel).toHaveBeenCalledOnce();
  });
});
