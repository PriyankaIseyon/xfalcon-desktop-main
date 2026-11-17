import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { FileDiffViewer } from "./FileDiffViewer";

const source = '--- a/data.json\n+++ b/data.json\n@@ -1 +1 @@\n-{"value":1}\n+{"value":2}\n';

afterEach(cleanup);

describe("FileDiffViewer", () => {
  it("shows line numbers and non-color change markers", () => {
    render(<FileDiffViewer source={source} />);

    expect(screen.getByLabelText("Deleted line 1")).toHaveTextContent('-{"value":1}');
    expect(screen.getByLabelText("Added line 1")).toHaveTextContent('+{"value":2}');
  });

  it("switches between unified and split layouts", async () => {
    const user = userEvent.setup();
    render(<FileDiffViewer source={source} />);

    await user.click(screen.getByRole("radio", { name: "Split" }));

    expect(screen.getByTestId("split-diff")).toBeInTheDocument();
  });

  it("states when a binary diff cannot be rendered as lines", () => {
    render(<FileDiffViewer source={"Binary files a/data.bin and b/data.bin differ\n"} />);

    expect(screen.getByText("Binary files a/data.bin and b/data.bin differ")).toBeInTheDocument();
  });

  it("names the one file in its region label", () => {
    render(<FileDiffViewer source={source} />);

    expect(screen.getByRole("region", { name: "Changes to data.json" })).toBeInTheDocument();
  });

  it("falls back to a generic region label for a multi-file plan", () => {
    const multiFile = [
      "--- a/data.json",
      "+++ b/data.json",
      "@@ -1 +1 @@",
      '-{"value":1}',
      '+{"value":2}',
      "--- a/notes.md",
      "+++ b/notes.md",
      "@@ -1 +1 @@",
      "-old",
      "+new",
      "",
    ].join("\n");
    render(<FileDiffViewer source={multiFile} />);

    expect(screen.getByRole("region", { name: "File changes" })).toBeInTheDocument();
  });
});
