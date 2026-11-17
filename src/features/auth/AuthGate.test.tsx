import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppBootstrap, ReportView } from "../../lib/generated/contracts";
import { applyReportList, getReportsSnapshot } from "../reports/reportsStore";
import { AuthGate } from "./AuthGate";
import { resetAuthSessionForTests, setAuthSession } from "./authStore";
import { setSignedOutNotice } from "./signedOutNotice";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

const bootstrap: AppBootstrap = {
  version: 1,
  application_version: "0.1.0",
  platform: "windows",
  session: { status: "unauthenticated", version: 1 },
};

beforeEach(() => {
  resetAuthSessionForTests();
});

/** A promise this test settles by hand, to hold a sign-in open. */
function deferred<T>() {
  let settle!: (value: T) => void;
  let fail!: (reason: unknown) => void;
  const promise = new Promise<T>((resolve, reject) => {
    settle = resolve;
    fail = reject;
  });
  // An unhandled rejection warning would fire before the code under test
  // reaches its `catch`, so the promise always has a reader.
  promise.catch(() => {});
  return { promise, settle, fail };
}

afterEach(() => {
  cleanup();
  invokeMock.mockReset();
});

describe("AuthGate", () => {
  it("shows only the login surface when the session is not authenticated", () => {
    render(
      <AuthGate bootstrap={bootstrap}>
        <p>Secret workspace</p>
      </AuthGate>,
    );

    expect(screen.getByRole("heading", { name: "Sign in to xFalcon" })).toBeInTheDocument();
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    expect(screen.queryByText("Secret workspace")).not.toBeInTheDocument();
  });

  it("renders children when the session is authenticated", () => {
    setAuthSession({
      status: "authenticated",
      version: 1,
      identity: {
        version: 1,
        user_id: "user-1",
        tenant_id: "tenant-1",
        display_name: "Test User",
        email: null,
      },
      expires_at_ms: Date.now() + 60_000,
    });

    render(
      <AuthGate bootstrap={bootstrap}>
        <p>Secret workspace</p>
      </AuthGate>,
    );

    expect(screen.getByText("Secret workspace")).toBeInTheDocument();
  });

  it("shows the signed-out notice on the sign-in screen", () => {
    setSignedOutNotice("Signed out after 12 hours of inactivity. Sign in to continue.");

    render(
      <AuthGate bootstrap={bootstrap}>
        <p>Secret workspace</p>
      </AuthGate>,
    );

    expect(screen.getByText("Signed out")).toBeInTheDocument();
    expect(screen.getByText("Signed out after 12 hours of inactivity. Sign in to continue.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue in browser" })).toBeInTheDocument();
  });

  it("does not show the signed-out notice once the person is signed in", () => {
    setSignedOutNotice("Signed out after 12 hours of inactivity. Sign in to continue.");
    setAuthSession({
      status: "authenticated",
      version: 1,
      identity: { version: 1, user_id: "user-1", tenant_id: "tenant-1", display_name: "Test User", email: null },
      expires_at_ms: Date.now() + 60_000,
    });

    render(
      <AuthGate bootstrap={bootstrap}>
        <p>Secret workspace</p>
      </AuthGate>,
    );

    expect(screen.queryByText("Signed out")).not.toBeInTheDocument();
    expect(screen.getByText("Secret workspace")).toBeInTheDocument();
  });

  it("clears report metadata before a different account can render", () => {
    const session = (userId: string) => ({
      status: "authenticated" as const,
      version: 1 as const,
      identity: {
        version: 1 as const,
        user_id: userId,
        tenant_id: "tenant-1",
        display_name: userId,
        email: null,
      },
      expires_at_ms: Date.now() + 60_000,
    });
    const privateReport: ReportView = {
      version: 1,
      id: "report-private",
      kind: "dashboard",
      title: "Acquisition plan",
      description: "Account A only",
      created_at_ms: 1n,
      updated_at_ms: 1n,
      thumbnail_updated_at_ms: null,
      data_sources: [],
      pinned: false,
      hidden: false,
      byte_size: 1n,
      scope: "user",
      is_owner: true,
      owner_display_name: null,
      refreshable: true,
      can_refresh: true,
      can_manage: true,
      can_delete: true,
      page_count: 1,
      dataset_count: 1,
      source_conversation_id: "conv-a",
    };

    setAuthSession(session("user-a"));
    applyReportList([privateReport]);
    expect(getReportsSnapshot().reports).toEqual([privateReport]);

    setAuthSession(session("user-b"));

    expect(getReportsSnapshot().reports).toEqual([]);
    expect(getReportsSnapshot().status).toBe("loading");
  });

  it("invokes native login without persisting secrets in the store", async () => {
    const technicalError = "auth is not configured for this build at https://issuer.example/internal";
    invokeMock.mockRejectedValueOnce(new Error(technicalError));
    const user = userEvent.setup();

    render(
      <AuthGate bootstrap={bootstrap}>
        <p>Secret workspace</p>
      </AuthGate>,
    );

    await user.click(screen.getByRole("button", { name: "Continue in browser" }));
    expect(invokeMock).toHaveBeenCalledWith("auth_login");
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Sign-in is unavailable. Contact the administrator, then try again.",
    );
    expect(screen.getByRole("alert")).not.toHaveTextContent(technicalError);
  });

  it("names the reason a sign-in was refused, from the code and not the message", async () => {
    invokeMock.mockRejectedValueOnce({ version: 1, code: "loopback_unavailable" });
    const user = userEvent.setup();

    render(
      <AuthGate bootstrap={bootstrap}>
        <p>Secret workspace</p>
      </AuthGate>,
    );

    await user.click(screen.getByRole("button", { name: "Continue in browser" }));

    expect(screen.getByRole("alert")).toHaveTextContent(
      "The port sign-in needs is in use on this device. Close other xFalcon windows, then try again.",
    );
    expect(screen.getByRole("alert")).not.toHaveTextContent("administrator");
  });

  it("cancels a sign-in that is still waiting for the browser, without reporting it as a failure", async () => {
    const login = deferred<never>();
    invokeMock.mockImplementationOnce(() => login.promise);
    invokeMock.mockResolvedValueOnce({ status: "unauthenticated", version: 1 });
    const user = userEvent.setup();

    render(
      <AuthGate bootstrap={bootstrap}>
        <p>Secret workspace</p>
      </AuthGate>,
    );

    await user.click(screen.getByRole("button", { name: "Continue in browser" }));
    await user.click(screen.getByRole("button", { name: "Cancel sign-in" }));

    expect(invokeMock).toHaveBeenCalledWith("auth_cancel");
    // The abandoned flow rejects after the cancel — the backend reports the
    // cancel as the reason. A person who just cancelled is not told that
    // cancelling failed.
    await act(async () => {
      login.fail({ version: 1, code: "cancelled" });
    });
    expect(screen.getByRole("button", { name: "Continue in browser" })).toBeEnabled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("keeps the primary action visibly pending while the browser flow is open", async () => {
    invokeMock.mockReturnValueOnce(new Promise(() => {}));
    const user = userEvent.setup();

    render(
      <AuthGate bootstrap={bootstrap}>
        <p>Secret workspace</p>
      </AuthGate>,
    );

    await user.click(screen.getByRole("button", { name: "Continue in browser" }));

    const pending = screen.getByRole("button", { name: "Waiting for browser…" });
    expect(pending).toBeDisabled();
    expect(pending).toHaveAttribute("aria-busy", "true");
  });

  it("carries the brand mark and an escape when bootstrap itself lands on an in-progress sign-in", async () => {
    // Reachable on a restart, not just a fresh click: the backend keeps
    // "authenticating" across relaunch while its loopback listener is still
    // open, so this can be the very first thing `AuthGate` renders --
    // `LoginView`'s own Cancel button is local state that never mounted.
    setAuthSession({ status: "authenticating", version: 1 });
    invokeMock.mockResolvedValueOnce({ status: "unauthenticated", version: 1 });
    const user = userEvent.setup();

    render(
      <AuthGate bootstrap={bootstrap}>
        <p>Secret workspace</p>
      </AuthGate>,
    );

    expect(screen.getByRole("heading", { name: "Continuing sign-in…" })).toHaveClass("type-title", "font-semibold");
    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("Opening secure sign-in…");
    expect(status).toHaveClass("type-body", "text-secondary");

    await user.click(screen.getByRole("button", { name: "Cancel sign-in" }));

    expect(invokeMock).toHaveBeenCalledWith("auth_cancel");
    expect(screen.getByRole("heading", { name: "Sign in to xFalcon" })).toBeInTheDocument();
  });
});
