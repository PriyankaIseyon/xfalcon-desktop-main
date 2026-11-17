import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { diagnosticsLogRendererError } from "../../lib/native";
import { ChatErrorBoundary } from "./ChatErrorBoundary";

vi.mock("../../lib/native", () => ({
  diagnosticsLogRendererError: vi.fn().mockResolvedValue(undefined),
}));

afterEach(cleanup);

beforeEach(() => {
  vi.mocked(diagnosticsLogRendererError).mockClear();
});

describe("ChatErrorBoundary", () => {
  it("isolates chat render failures from the rest of the shell and logs to diagnostics", async () => {
    let fail = true;
    function BrokenChat() {
      if (fail) throw new Error("secret-token");
      return <p>Chat restored</p>;
    }

    const user = userEvent.setup();
    render(
      <ChatErrorBoundary
        onReset={() => {
          fail = false;
        }}
      >
        <BrokenChat />
      </ChatErrorBoundary>,
    );

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("Chat could not be displayed");
    expect(alert).not.toHaveTextContent("secret-token");
    expect(diagnosticsLogRendererError).toHaveBeenCalledWith(
      expect.objectContaining({
        version: 1,
        message: expect.stringContaining("Error: secret-token"),
      }),
    );

    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(screen.getByText("Chat restored")).toBeInTheDocument();
  });

  it("reports a repeated failure and clears it when the conversation changes", async () => {
    let fail = true;
    function BrokenChat() {
      if (fail) throw new Error("render failure");
      return <p>Next chat restored</p>;
    }

    const user = userEvent.setup();
    const view = render(
      <ChatErrorBoundary resetKey="conversation-1">
        <BrokenChat />
      </ChatErrorBoundary>,
    );

    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("Chat still could not be displayed")).toBeInTheDocument();

    fail = false;
    view.rerender(
      <ChatErrorBoundary resetKey="conversation-2">
        <BrokenChat />
      </ChatErrorBoundary>,
    );

    expect(await screen.findByText("Next chat restored")).toBeInTheDocument();
    expect(diagnosticsLogRendererError).toHaveBeenCalledTimes(2);
  });
});
