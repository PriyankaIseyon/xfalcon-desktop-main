import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { CanvasSlot } from "./CanvasSlot";

afterEach(cleanup);

describe("CanvasSlot", () => {
  it("consumes no DOM when closed", () => {
    const { container } = render(
      <CanvasSlot open={false}>
        <p>Artifact preview</p>
      </CanvasSlot>,
    );

    expect(container).toBeEmptyDOMElement();
  });

  it("exposes an accessible complementary region when open", () => {
    render(
      <CanvasSlot open ariaLabel="Artifact">
        <p>Artifact preview</p>
      </CanvasSlot>,
    );

    expect(screen.getByRole("complementary", { name: "Artifact" })).toHaveTextContent("Artifact preview");
  });
});
