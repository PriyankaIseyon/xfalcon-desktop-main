import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { WorkspaceSkeleton } from "./WorkspaceSkeleton";

describe("WorkspaceSkeleton", () => {
  it("announces loading and holds the shell frame with shared skeletons", () => {
    const { container } = render(<WorkspaceSkeleton />);
    expect(screen.getByRole("status")).toBeInTheDocument();
    expect(screen.getByText("Loading workspace…")).toBeInTheDocument();
    // Shared skeletons carry the shimmer; reduced-motion handling is CSS-side
    // (`@media (prefers-reduced-motion: reduce)` disables the animation).
    expect(container.querySelectorAll(".skeleton-shimmer").length).toBeGreaterThan(5);
  });
});
