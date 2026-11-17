import React from "react";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { IconButton } from "./IconButton";
import { Tooltip } from "./Tooltip";
import { TruncatedText } from "./TruncatedText";

afterEach(cleanup);

/** jsdom gives every box a zero size, so the panel would measure 0x0 and every
 * placement would collapse onto the anchor's own corner. */
const TIP_WIDTH = 80;
const TIP_HEIGHT = 20;

beforeEach(() => {
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", { configurable: true, value: TIP_WIDTH });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, value: TIP_HEIGHT });
});

function anchorAt(node: HTMLElement, box: { top: number; left: number; width: number; height: number }) {
  node.getBoundingClientRect = () =>
    ({
      top: box.top,
      bottom: box.top + box.height,
      left: box.left,
      right: box.left + box.width,
      width: box.width,
      height: box.height,
      x: box.left,
      y: box.top,
      toJSON: () => ({}),
    }) as DOMRect;
}

describe("Tooltip", () => {
  it("adds no element of its own around the trigger", () => {
    render(
      <div data-testid="row">
        <Tooltip label="Copy message">
          <button type="button">Copy</button>
        </Tooltip>
      </div>,
    );
    // An `inline-flex` wrapper here would make a tooltipped control lay out
    // differently from a bare one in the same flex row.
    expect(screen.getByRole("button").parentElement).toBe(screen.getByTestId("row"));
  });

  it("renders the panel into the body, so an overflow ancestor cannot clip it", async () => {
    const user = userEvent.setup();
    render(
      <div data-testid="clipper" style={{ overflow: "hidden" }}>
        <Tooltip delay={0} label="Copy message">
          <button type="button">Copy</button>
        </Tooltip>
      </div>,
    );

    await user.hover(screen.getByRole("button"));
    const tip = await screen.findByRole("tooltip");
    expect(tip).toHaveTextContent("Copy message");
    expect(tip.parentElement).toBe(document.body);
    expect(screen.getByTestId("clipper")).not.toContainElement(tip);
  });

  it("hides again when the pointer leaves", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip delay={0} label="Copy message">
        <button type="button">Copy</button>
      </Tooltip>,
    );
    const button = screen.getByRole("button");

    await user.hover(button);
    await screen.findByRole("tooltip");
    await user.unhover(button);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("shows on keyboard focus and closes on Escape", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip label="Copy message">
        <button type="button">Copy</button>
      </Tooltip>,
    );

    await user.tab();
    expect(screen.getByRole("button")).toHaveFocus();
    // Focus shows it at once: a keyboard user has no hover to rest.
    expect(screen.getByRole("tooltip")).toBeInTheDocument();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("gets out of the way of the click that acted on the control", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip delay={0} label="Copy message">
        <button type="button">Copy</button>
      </Tooltip>,
    );

    await user.hover(screen.getByRole("button"));
    await screen.findByRole("tooltip");
    await user.click(screen.getByRole("button"));
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("closes when the page scrolls out from under the anchor", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip delay={0} label="Copy message">
        <button type="button">Copy</button>
      </Tooltip>,
    );

    await user.hover(screen.getByRole("button"));
    await screen.findByRole("tooltip");
    act(() => {
      window.dispatchEvent(new Event("scroll"));
    });
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("places the panel on the side it was asked for", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip delay={0} label="Copy message" side="bottom">
        <button type="button">Copy</button>
      </Tooltip>,
    );
    const button = screen.getByRole("button");
    anchorAt(button, { top: 300, left: 100, width: 40, height: 20 });

    await user.hover(button);
    const tip = await screen.findByRole("tooltip");
    // 6 px below the anchor's bottom edge, centred on it.
    expect(tip.style.top).toBe("326px");
    expect(tip.style.left).toBe(`${100 + (40 - TIP_WIDTH) / 2}px`);
    expect(tip.style.visibility).toBe("visible");
  });

  it("flips to the opposite side rather than leaving the viewport", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip delay={0} label="Copy message" side="top">
        <button type="button">Copy</button>
      </Tooltip>,
    );
    const button = screen.getByRole("button");
    // Hard against the top edge: above would put the panel at -16.
    anchorAt(button, { top: 10, left: 100, width: 40, height: 20 });

    await user.hover(button);
    const tip = await screen.findByRole("tooltip");
    expect(tip.style.top).toBe("36px");
  });

  it("clamps a panel that would run off the left edge", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip delay={0} label="Copy message" side="top">
        <button type="button">Copy</button>
      </Tooltip>,
    );
    const button = screen.getByRole("button");
    anchorAt(button, { top: 300, left: 0, width: 20, height: 20 });

    await user.hover(button);
    const tip = await screen.findByRole("tooltip");
    expect(tip.style.left).toBe("8px");
  });

  it("reaches the DOM node through a component that spreads its props", async () => {
    const user = userEvent.setup();
    // `Chip` and friends take no ref of their own; they hand the rest of their
    // props to a DOM node. If the anchor ref did not arrive there, the panel
    // would mount unpositioned and stay invisible.
    function Spread(props: React.HTMLAttributes<HTMLSpanElement>) {
      return <span {...props} />;
    }
    render(
      <Tooltip delay={0} label="src/features/chat/MessageList.tsx">
        <Spread>chat/MessageList.tsx</Spread>
      </Tooltip>,
    );
    const chip = screen.getByText("chat/MessageList.tsx");
    anchorAt(chip, { top: 300, left: 100, width: 40, height: 20 });

    await user.hover(chip);
    const tip = await screen.findByRole("tooltip");
    expect(tip.style.visibility).toBe("visible");
    expect(tip.style.top).toBe(`${300 - TIP_HEIGHT - 6}px`);
  });

  it("describes the control only when the tooltip says something new", async () => {
    const user = userEvent.setup();
    render(
      <>
        <Tooltip delay={0} label="Copy this message to the clipboard">
          <button aria-label="Copy" type="button" />
        </Tooltip>
        <Tooltip delay={0} label="Copy">
          <button aria-label="Copy" data-testid="repeat" type="button" />
        </Tooltip>
      </>,
    );

    const [adds, repeats] = screen.getAllByRole("button");
    await user.hover(adds!);
    await waitFor(() => expect(adds).toHaveAttribute("aria-describedby"));

    await user.hover(repeats!);
    await screen.findByRole("tooltip");
    // A tooltip that only repeats the name would be read twice.
    expect(repeats).not.toHaveAttribute("aria-describedby");
  });
});

describe("IconButton help", () => {
  it("shows its accessible name without the call site repeating itself", async () => {
    const user = userEvent.setup();
    render(<IconButton aria-label="Delete report">x</IconButton>);

    await user.hover(screen.getByRole("button"));
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Delete report");
  });

  it("prefers a hint that says more than the name", async () => {
    const user = userEvent.setup();
    render(
      <IconButton aria-label="Expand to full width" tooltip="Full width (Esc to leave)">
        x
      </IconButton>,
    );

    await user.hover(screen.getByRole("button"));
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Full width (Esc to leave)");
  });

  it("takes null for a control that must stay silent", async () => {
    const user = userEvent.setup();
    render(
      <IconButton aria-label="Delete report" tooltip={null}>
        x
      </IconButton>,
    );

    await user.hover(screen.getByRole("button"));
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("never carries a native title", () => {
    render(<IconButton aria-label="Delete report">x</IconButton>);
    expect(screen.getByRole("button")).not.toHaveAttribute("title");
  });
});

describe("TruncatedText", () => {
  it("says nothing extra while the text fits", async () => {
    const user = userEvent.setup();
    render(<TruncatedText>Quarterly review</TruncatedText>);
    const node = screen.getByText("Quarterly review");

    await user.hover(node);
    expect(node).not.toHaveAttribute("title");
  });

  it("reveals the whole string once the box clips it", async () => {
    const user = userEvent.setup();
    render(<TruncatedText>Quarterly review</TruncatedText>);
    const node = screen.getByText("Quarterly review");
    Object.defineProperty(node, "clientWidth", { configurable: true, value: 40 });
    Object.defineProperty(node, "scrollWidth", { configurable: true, value: 400 });

    await user.hover(node);
    expect(node).toHaveAttribute("title", "Quarterly review");
  });

  it("prefers the string it was given over what it renders", async () => {
    const user = userEvent.setup();
    render(<TruncatedText text="2,500.00">2,500</TruncatedText>);
    const node = screen.getByText("2,500");
    Object.defineProperty(node, "clientWidth", { configurable: true, value: 10 });
    Object.defineProperty(node, "scrollWidth", { configurable: true, value: 100 });

    await user.hover(node);
    expect(node).toHaveAttribute("title", "2,500.00");
  });

  it("reveals the whole string of an abbreviated text even when what is shown fits", async () => {
    const user = userEvent.setup();
    render(
      <TruncatedText abbreviated text="Global Geography Trivia Dashboard for Review.html">
        Global Geography…Review.html
      </TruncatedText>,
    );
    const node = screen.getByText("Global Geography…Review.html");

    await user.hover(node);
    expect(node).toHaveAttribute("title", "Global Geography Trivia Dashboard for Review.html");
  });

  it("takes the tag it should render, and keeps its own truncation class", () => {
    render(
      <TruncatedText as="strong" className="sidebar-row__title">
        Quarterly review
      </TruncatedText>,
    );
    const node = screen.getByText("Quarterly review");
    expect(node.tagName).toBe("STRONG");
    expect(node).toHaveClass("truncate", "sidebar-row__title");
  });

  it("still calls a pointer handler the caller passed", async () => {
    const user = userEvent.setup();
    const onPointerEnter = vi.fn();
    render(<TruncatedText onPointerEnter={onPointerEnter}>Quarterly review</TruncatedText>);

    await user.hover(screen.getByText("Quarterly review"));
    expect(onPointerEnter).toHaveBeenCalledTimes(1);
  });
});
