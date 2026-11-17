import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { OfflineBanner } from "./OfflineBanner";

afterEach(cleanup);

describe("OfflineBanner", () => {
  it("stays out of the tree when connectivity is present", () => {
    const { container } = render(<OfflineBanner visible={false} />);
    expect(container).toBeEmptyDOMElement();
  });

  it("states offline as a warning, not an error -- the app still works, degraded", () => {
    render(<OfflineBanner visible />);

    const banner = screen.getByRole("status");
    expect(banner).toHaveAttribute("data-tone", "warn");
    expect(banner).toHaveTextContent("You appear to be offline");
    expect(banner).toHaveTextContent(
      "Actions that need the secure runtime or network may not complete until connectivity returns.",
    );
  });

  it("defers to the browser's own signal when no explicit visibility is given", () => {
    const online = Object.getOwnPropertyDescriptor(navigator, "onLine");
    try {
      Object.defineProperty(navigator, "onLine", { configurable: true, value: false });
      const { unmount } = render(<OfflineBanner />);
      expect(screen.getByRole("status")).toHaveTextContent("You appear to be offline");
      unmount();

      Object.defineProperty(navigator, "onLine", { configurable: true, value: true });
      const { container } = render(<OfflineBanner />);
      expect(container).toBeEmptyDOMElement();
    } finally {
      if (online) Object.defineProperty(navigator, "onLine", online);
    }
  });
});
