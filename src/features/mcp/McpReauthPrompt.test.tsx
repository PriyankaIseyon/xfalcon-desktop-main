import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { McpConnectionReport, McpReauthRequired } from "../../lib/generated/contracts";
import { Toaster } from "../../ui/Toaster";
import { resetToastsForTests } from "../../ui/toastStore";
import { McpReauthPrompt } from "./McpReauthPrompt";

const mcpServerSignInMock = vi.hoisted(() => vi.fn());
const mcpServerTestMock = vi.hoisted(() => vi.fn());
const onMcpReauthRequiredMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  mcpServerSignIn: mcpServerSignInMock,
  mcpServerTest: mcpServerTestMock,
  onMcpReauthRequired: onMcpReauthRequiredMock,
}));

const unlisten = vi.hoisted(() => vi.fn());
let raise: ((prompt: McpReauthRequired) => void) | null = null;

const refused: McpReauthRequired = {
  version: 1,
  id: "srv-ida",
  label: "IDA Cache",
  reason: "credential_refused",
};

const connected: McpConnectionReport = {
  version: 1,
  id: "srv-ida",
  state: "connected",
  tools: [],
  message: null,
};

function arm() {
  onMcpReauthRequiredMock.mockImplementation((handler: (prompt: McpReauthRequired) => void) => {
    raise = handler;
    return Promise.resolve(unlisten);
  });
}

async function renderAndRaise(prompt: McpReauthRequired = refused) {
  render(
    <>
      <McpReauthPrompt />
      <Toaster />
    </>,
  );
  await waitFor(() => expect(raise).not.toBeNull());
  await act(async () => {
    raise?.(prompt);
  });
}

afterEach(() => {
  cleanup();
  resetToastsForTests();
  mcpServerSignInMock.mockReset();
  mcpServerTestMock.mockReset();
  onMcpReauthRequiredMock.mockReset();
  unlisten.mockReset();
  raise = null;
});

describe("McpReauthPrompt", () => {
  it("shows nothing until a server is actually refused", async () => {
    arm();
    render(<McpReauthPrompt />);

    await waitFor(() => expect(onMcpReauthRequiredMock).toHaveBeenCalled());
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("names the server and says a sign-in is the fix", async () => {
    arm();
    await renderAndRaise();

    expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
    expect(screen.getByText("Reconnect IDA Cache")).toBeInTheDocument();
    expect(screen.getByText(/session has expired or requires authorization/i)).toBeInTheDocument();
  });

  it("says the different thing when the sign-in is merely too narrow", async () => {
    arm();
    await renderAndRaise({ ...refused, reason: "insufficient_scope" });

    expect(await screen.findByText(/does not cover what it now requires/i)).toBeInTheDocument();
  });

  it("reconnecting signs in and then verifies, rather than assuming it worked", async () => {
    arm();
    mcpServerSignInMock.mockResolvedValueOnce({});
    mcpServerTestMock.mockResolvedValueOnce(connected);
    const user = userEvent.setup();
    await renderAndRaise();

    await user.click(await screen.findByRole("button", { name: /^reconnect$/i }));

    await waitFor(() => expect(mcpServerSignInMock).toHaveBeenCalledWith("srv-ida", null, null));
    await waitFor(() => expect(mcpServerTestMock).toHaveBeenCalledWith("srv-ida"));
    // Verified connected, so the prompt has done its job and closes.
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(await screen.findByText("IDA Cache is connected again.")).toBeInTheDocument();
  });

  it("requests client details only when registration requires them, then verifies reconnection", async () => {
    arm();
    mcpServerSignInMock
      .mockRejectedValueOnce({ version: 1, code: "client_registration_unavailable" })
      .mockRejectedValueOnce({ version: 1, code: "sign_in_failed" })
      .mockResolvedValueOnce({});
    mcpServerTestMock.mockResolvedValueOnce(connected);
    const user = userEvent.setup();
    await renderAndRaise();
    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    expect(await screen.findByRole("dialog")).toHaveTextContent("IDA Cache");
    expect(screen.getByRole("button", { name: "Sign in" })).toBeDisabled();
    await user.type(screen.getByLabelText("Client ID"), "  operator-client  ");
    expect(screen.getByLabelText("Client secret")).toHaveValue("");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByText("Sign-in did not complete. Try again.")).toBeVisible();
    expect(mcpServerTestMock).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(mcpServerSignInMock).toHaveBeenLastCalledWith("srv-ida", "operator-client", null);
    expect(mcpServerTestMock).toHaveBeenCalledWith("srv-ida");
  });

  it("passes a required client secret only to the selected server's sign-in", async () => {
    arm();
    mcpServerSignInMock
      .mockRejectedValueOnce({ version: 1, code: "client_registration_unavailable" })
      .mockResolvedValueOnce({});
    mcpServerTestMock.mockResolvedValueOnce(connected);
    const user = userEvent.setup();
    await renderAndRaise();
    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    await user.type(await screen.findByLabelText("Client ID"), "issued-client");
    await user.type(screen.getByLabelText("Client secret"), "issued-secret");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    await waitFor(() =>
      expect(mcpServerSignInMock).toHaveBeenLastCalledWith("srv-ida", "issued-client", "issued-secret"),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("does not carry client details into the next queued server", async () => {
    arm();
    mcpServerSignInMock.mockRejectedValue({ version: 1, code: "client_registration_unavailable" });
    const user = userEvent.setup();
    await renderAndRaise();
    await act(async () => {
      raise?.({ ...refused, id: "other", label: "Other server" });
    });
    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    await user.type(await screen.findByLabelText("Client ID"), "first-client");
    await user.type(screen.getByLabelText("Client secret"), "first-secret");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await screen.findByText("Reconnect Other server")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    expect(await screen.findByLabelText("Client ID")).toHaveValue("");
    expect(screen.getByLabelText("Client secret")).toHaveValue("");
    expect(mcpServerSignInMock).toHaveBeenLastCalledWith("other", null, null);
  });

  it("a sign-in that completes but leaves the server refused keeps the prompt up", async () => {
    // Closing here would report a fix that did not happen.
    arm();
    mcpServerSignInMock.mockResolvedValueOnce({});
    mcpServerTestMock.mockResolvedValueOnce({
      ...connected,
      state: "rejected",
      message: "This server rejected the saved sign-in. Sign in again to reconnect it.",
    });
    const user = userEvent.setup();
    await renderAndRaise();

    await user.click(await screen.findByRole("button", { name: /^reconnect$/i }));

    expect(await screen.findByText(/rejected the saved sign-in/i)).toBeInTheDocument();
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
  });

  it("does not accept a connection report for a different server", async () => {
    arm();
    mcpServerSignInMock.mockResolvedValueOnce({});
    mcpServerTestMock.mockResolvedValueOnce({ ...connected, id: "srv-other" });
    const user = userEvent.setup();
    await renderAndRaise();

    await user.click(await screen.findByRole("button", { name: "Reconnect" }));

    expect(await screen.findByText(/returned a different server/i)).toBeInTheDocument();
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
  });

  it("a cancelled sign-in leaves the server refused and says so", async () => {
    arm();
    mcpServerSignInMock.mockRejectedValueOnce({ version: 1, code: "sign_in_declined" });
    const user = userEvent.setup();
    await renderAndRaise();

    await user.click(await screen.findByRole("button", { name: /^reconnect$/i }));

    expect(await screen.findByText(/did not complete/i)).toBeInTheDocument();
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    expect(mcpServerTestMock).not.toHaveBeenCalled();
  });

  it("dismissing closes the prompt without pretending to have reconnected", async () => {
    arm();
    const user = userEvent.setup();
    await renderAndRaise();

    await user.click(await screen.findByRole("button", { name: /^dismiss$/i }));

    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(mcpServerSignInMock).not.toHaveBeenCalled();
  });

  it("a stray click on the scrim does not dismiss it", async () => {
    // A refused grant blocks every tool call on that server, so accepting the
    // degraded state must be a deliberate act.
    arm();
    const user = userEvent.setup();
    await renderAndRaise();
    await screen.findByRole("alertdialog");
    const scrim = document.querySelector(".dialog-scrim");
    expect(scrim).not.toBeNull();

    await user.click(scrim as Element);

    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
  });

  it("admits only one reconnect before the busy state renders", async () => {
    arm();
    let resolveSignIn!: () => void;
    mcpServerSignInMock.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          resolveSignIn = resolve;
        }),
    );
    mcpServerTestMock.mockResolvedValueOnce(connected);
    await renderAndRaise();
    const reconnect = await screen.findByRole("button", { name: "Reconnect" });

    reconnect.click();
    reconnect.click();

    expect(mcpServerSignInMock).toHaveBeenCalledTimes(1);
    resolveSignIn();
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  });

  it("a second refusal does not replace the server being acted on", async () => {
    arm();
    await renderAndRaise();
    await act(async () => {
      raise?.({ ...refused, id: "srv-other", label: "Other Server" });
    });

    expect(screen.getByText("Reconnect IDA Cache")).toBeInTheDocument();
    expect(screen.queryByText("Reconnect Other Server")).not.toBeInTheDocument();
  });

  it("queues distinct servers and deduplicates repeated failures", async () => {
    arm();
    const user = userEvent.setup();
    await renderAndRaise();
    await act(async () => {
      raise?.(refused);
      raise?.({ ...refused, id: "srv-other", label: "Other Server" });
      raise?.({ ...refused, id: "srv-other", label: "Other Server" });
    });
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.getByRole("alertdialog", { name: "Reconnect Other Server" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("refreshes connection consumers only after verified reconnection", async () => {
    arm();
    const refreshed = vi.fn();
    render(<McpReauthPrompt onReconnected={refreshed} />);
    await waitFor(() => expect(raise).not.toBeNull());
    await act(async () => {
      raise?.(refused);
    });
    mcpServerSignInMock.mockResolvedValueOnce({});
    mcpServerTestMock.mockResolvedValueOnce(connected);
    await userEvent.setup().click(screen.getByRole("button", { name: "Reconnect" }));
    await waitFor(() => expect(refreshed).toHaveBeenCalledTimes(1));
  });

  it("does not check or refresh a server after the prompt unmounts", async () => {
    arm();
    let finish!: () => void;
    mcpServerSignInMock.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    const refreshed = vi.fn();
    const { unmount } = render(<McpReauthPrompt onReconnected={refreshed} />);
    await waitFor(() => expect(raise).not.toBeNull());
    await act(async () => {
      raise?.(refused);
    });
    await userEvent.setup().click(screen.getByRole("button", { name: "Reconnect" }));
    unmount();
    await act(async () => {
      finish();
    });
    expect(mcpServerTestMock).not.toHaveBeenCalled();
    expect(refreshed).not.toHaveBeenCalled();
  });

  it("stops listening when unmounted", async () => {
    arm();
    const { unmount } = render(<McpReauthPrompt />);
    await waitFor(() => expect(raise).not.toBeNull());

    unmount();

    await waitFor(() => expect(unlisten).toHaveBeenCalled());
  });
});
