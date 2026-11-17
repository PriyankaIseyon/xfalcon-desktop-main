import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { resetPlatformForTests, setPlatform } from "../lib/platform";
import { Shortcut } from "./Kbd";

afterEach(() => {
  cleanup();
  resetPlatformForTests();
});

describe("Shortcut", () => {
  it("joins keys with a plus and spells the word modifier, off Apple", () => {
    setPlatform("windows");
    render(<Shortcut keys={["Mod", "Shift", "F"]} />);

    const shortcut = screen.getByRole("img", { name: "Control plus Shift plus F" });
    expect(shortcut).toHaveTextContent("Ctrl+Shift+F");
  });

  it("runs Apple's glyphs together with no separator, and spells them Command/Shift", () => {
    setPlatform("macos");
    render(<Shortcut keys={["Mod", "Shift", "F"]} />);

    const shortcut = screen.getByRole("img", { name: "Command plus Shift plus F" });
    expect(shortcut).toHaveTextContent("⌘⇧F");
  });

  it("hides every key cap from assistive tech so only the one accessible name is read", () => {
    setPlatform("windows");
    const { container } = render(<Shortcut keys={["Mod", "K"]} />);

    const caps = container.querySelectorAll("kbd");
    expect(caps).toHaveLength(2);
    for (const cap of caps) {
      expect(cap).toHaveAttribute("aria-hidden", "true");
    }
  });

  it("falls back to Ctrl on the unknown platform, so a shortcut is never blank before bootstrap answers", () => {
    render(<Shortcut keys={["Mod", "K"]} />);
    expect(screen.getByRole("img", { name: "Control plus K" })).toHaveTextContent("Ctrl+K");
  });

  it("prints a key with no known spelling as itself, on any platform", () => {
    setPlatform("linux");
    render(<Shortcut keys={["Q"]} />);
    expect(screen.getByRole("img", { name: "Q" })).toHaveTextContent("Q");
  });
});
