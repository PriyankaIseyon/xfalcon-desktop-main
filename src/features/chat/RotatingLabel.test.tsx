import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { RotatingLabel } from "./RotatingLabel";
import { LONGEST_THINKING_PHRASE, THINKING_PHRASES } from "./thinkingPhrase";

afterEach(cleanup);

describe("RotatingLabel", () => {
  it("reserves the width of the longest phrase whichever phrase is showing", () => {
    for (const phrase of THINKING_PHRASES) {
      const { unmount } = render(<RotatingLabel phrase={phrase} />);
      expect(screen.getByText(phrase)).toHaveAttribute("data-reserve", LONGEST_THINKING_PHRASE);
      unmount();
    }
  });

  it("names the longest phrase of the cycle", () => {
    for (const phrase of THINKING_PHRASES) {
      expect(LONGEST_THINKING_PHRASE.length).toBeGreaterThanOrEqual(phrase.length);
    }
  });
});
