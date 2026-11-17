import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatgptPlanStatusView, ClaudeRuntimeStatusView } from "../../lib/generated/contracts";
import { useChatgptPlan, useClaudeRuntime } from "./usePlanProviders";

const mocks = vi.hoisted(() => ({
  claudeRuntimeStatus: vi.fn(),
  claudeRuntimeSetEnabled: vi.fn(),
  claudeRuntimeSetEffort: vi.fn(),
  chatgptPlanStatus: vi.fn(),
  chatgptPlanSignIn: vi.fn(),
  chatgptPlanSignInCancel: vi.fn(),
  chatgptPlanSignOut: vi.fn(),
  chatgptPlanWelcomeAck: vi.fn(),
}));

vi.mock("../../lib/native", () => mocks);

const claudeOff: ClaudeRuntimeStatusView = {
  version: 1,
  enabled: false,
  state: "disabled",
  installed_version: null,
  required_version: "2.1.295",
  plan: null,
  sign_in_method: null,
  sign_in_command: "claude auth login",
  effort: "high",
};

const claudeReady: ClaudeRuntimeStatusView = {
  ...claudeOff,
  enabled: true,
  state: "ready",
  installed_version: "2.1.295",
  plan: "Max",
  sign_in_method: "Claude subscription",
};

const chatgptSignedOut: ChatgptPlanStatusView = {
  version: 1,
  state: "signed_out",
  welcome_pending: false,
  account_label: null,
  model_count: 0,
  usage_url: "https://chatgpt.com/settings/usage",
  error: null,
};

const chatgptReady: ChatgptPlanStatusView = {
  ...chatgptSignedOut,
  state: "ready",
  welcome_pending: true,
  account_label: "Work account",
  model_count: 2,
};

// The text the backend sends after a sign-out that OpenAI did not confirm (src-tauri plan_providers/service.rs).
const SIGN_OUT_UNCONFIRMED =
  "Signed out here, but OpenAI did not confirm it. You can also disconnect xFalcon in your ChatGPT settings.";

beforeEach(() => {
  for (const mock of Object.values(mocks)) mock.mockReset();
});

describe("useClaudeRuntime", () => {
  it("loads the status on mount and clears busy once it lands", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(claudeOff);

    const { result } = renderHook(() => useClaudeRuntime());

    expect(result.current.busy).toBe(true);
    await waitFor(() => expect(result.current.status).toEqual(claudeOff));
    expect(result.current.busy).toBe(false);
    expect(result.current.error).toBeNull();
    expect(mocks.claudeRuntimeStatus).toHaveBeenCalledOnce();
  });

  it("reports a failed load as an error with no status, and does not stay busy", async () => {
    mocks.claudeRuntimeStatus.mockRejectedValueOnce({ version: 1, code: "unexpected" });

    const { result } = renderHook(() => useClaudeRuntime());

    await waitFor(() => expect(result.current.error).toBe("Could not check Claude Code. Try again."));
    expect(result.current.status).toBeNull();
    expect(result.current.busy).toBe(false);
  });

  it("stores the status that a successful enable returns and reports success", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(claudeOff);
    mocks.claudeRuntimeSetEnabled.mockResolvedValueOnce(claudeReady);
    const { result } = renderHook(() => useClaudeRuntime());
    await waitFor(() => expect(result.current.status).toEqual(claudeOff));

    let saved = false;
    await act(async () => {
      saved = await result.current.setEnabled(true);
    });

    expect(saved).toBe(true);
    expect(mocks.claudeRuntimeSetEnabled).toHaveBeenCalledWith({ enabled: true });
    expect(result.current.status).toEqual(claudeReady);
    expect(result.current.busy).toBe(false);
  });

  it("keeps the previous status and reports the failure when enabling fails", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(claudeOff);
    mocks.claudeRuntimeSetEnabled.mockRejectedValueOnce(new Error("save failed"));
    const { result } = renderHook(() => useClaudeRuntime());
    await waitFor(() => expect(result.current.status).toEqual(claudeOff));

    let saved = true;
    await act(async () => {
      saved = await result.current.setEnabled(true);
    });

    expect(saved).toBe(false);
    expect(result.current.status).toEqual(claudeOff);
    expect(result.current.error).toBe("The Claude setting could not be saved. Try again.");
    expect(result.current.busy).toBe(false);
  });

  it("stays busy until the last of two overlapping commands settles", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(claudeOff);
    let finishRefresh!: (value: ClaudeRuntimeStatusView) => void;
    mocks.claudeRuntimeStatus.mockReturnValueOnce(
      new Promise<ClaudeRuntimeStatusView>((resolve) => {
        finishRefresh = resolve;
      }),
    );
    mocks.claudeRuntimeSetEnabled.mockResolvedValueOnce(claudeReady);
    const { result } = renderHook(() => useClaudeRuntime());
    await waitFor(() => expect(result.current.status).toEqual(claudeOff));

    let refreshing!: Promise<boolean>;
    await act(async () => {
      refreshing = result.current.refresh();
      await result.current.setEnabled(true);
    });
    expect(result.current.busy).toBe(true);

    await act(async () => {
      finishRefresh(claudeOff);
      await refreshing;
    });
    expect(result.current.busy).toBe(false);
  });

  it("still reports a command that settles after unmount, and writes nothing", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(claudeOff);
    let finishEnable!: (value: ClaudeRuntimeStatusView) => void;
    mocks.claudeRuntimeSetEnabled.mockReturnValueOnce(
      new Promise<ClaudeRuntimeStatusView>((resolve) => {
        finishEnable = resolve;
      }),
    );
    const { result, unmount } = renderHook(() => useClaudeRuntime());
    await waitFor(() => expect(result.current.status).toEqual(claudeOff));

    let pending!: Promise<boolean>;
    await act(async () => {
      pending = result.current.setEnabled(true);
    });
    unmount();
    finishEnable(claudeReady);

    // The change happened on this device, so the caller still learns of it.
    await expect(pending).resolves.toBe(true);
    expect(result.current.status).toEqual(claudeOff);
  });

  it("stores the status that a successful effort change returns and reports success", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(claudeReady);
    mocks.claudeRuntimeSetEffort.mockResolvedValueOnce({ ...claudeReady, effort: "max" });
    const { result } = renderHook(() => useClaudeRuntime());
    await waitFor(() => expect(result.current.status).toEqual(claudeReady));

    let saved = false;
    await act(async () => {
      saved = await result.current.setEffort("max");
    });

    expect(saved).toBe(true);
    expect(mocks.claudeRuntimeSetEffort).toHaveBeenCalledWith({ effort: "max" });
    expect(result.current.status).toEqual({ ...claudeReady, effort: "max" });
    expect(result.current.error).toBeNull();
    expect(result.current.busy).toBe(false);
  });

  it("keeps the previous status and reports the failure when an effort change fails", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(claudeReady);
    mocks.claudeRuntimeSetEffort.mockRejectedValueOnce({ version: 1, code: "unexpected" });
    const { result } = renderHook(() => useClaudeRuntime());
    await waitFor(() => expect(result.current.status).toEqual(claudeReady));

    let saved = true;
    await act(async () => {
      saved = await result.current.setEffort("max");
    });

    expect(saved).toBe(false);
    expect(result.current.status).toEqual(claudeReady);
    expect(result.current.error).toBe("The Claude setting could not be saved. Try again.");
    expect(result.current.busy).toBe(false);
  });
});

describe("useChatgptPlan", () => {
  it("loads the status on mount and clears busy once it lands", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptSignedOut);

    const { result } = renderHook(() => useChatgptPlan());

    expect(result.current.busy).toBe(true);
    await waitFor(() => expect(result.current.status).toEqual(chatgptSignedOut));
    expect(result.current.busy).toBe(false);
    expect(result.current.signingIn).toBe(false);
    expect(mocks.chatgptPlanStatus).toHaveBeenCalledOnce();
  });

  it("reports the sign-in as in progress until the browser step resolves with a view", async () => {
    mocks.chatgptPlanStatus.mockResolvedValue(chatgptSignedOut);
    let finishSignIn!: (value: ChatgptPlanStatusView) => void;
    mocks.chatgptPlanSignIn.mockReturnValueOnce(
      new Promise<ChatgptPlanStatusView>((resolve) => {
        finishSignIn = resolve;
      }),
    );
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status).toEqual(chatgptSignedOut));

    let signing!: Promise<boolean>;
    await act(async () => {
      signing = result.current.signIn();
    });
    expect(result.current.signingIn).toBe(true);
    expect(result.current.busy).toBe(true);

    await act(async () => {
      finishSignIn(chatgptReady);
      await signing;
    });

    await expect(signing).resolves.toBe(true);
    expect(result.current.signingIn).toBe(false);
    expect(result.current.busy).toBe(false);
    expect(result.current.status).toEqual(chatgptReady);
  });

  it("reports a failed sign-in as an error that the reader can retry", async () => {
    mocks.chatgptPlanStatus.mockResolvedValue(chatgptSignedOut);
    mocks.chatgptPlanSignIn.mockRejectedValueOnce({ version: 1, code: "unexpected" });
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status).toEqual(chatgptSignedOut));

    let saved = true;
    await act(async () => {
      saved = await result.current.signIn();
    });

    expect(saved).toBe(false);
    expect(result.current.error).toBe("Sign-in did not finish. Try again.");
    expect(result.current.signingIn).toBe(false);
    expect(result.current.busy).toBe(false);
  });

  it("does not report a sign-in that ends because of a cancel, and reads the status back", async () => {
    mocks.chatgptPlanStatus.mockResolvedValue(chatgptSignedOut);
    let rejectSignIn!: (cause: unknown) => void;
    mocks.chatgptPlanSignIn.mockReturnValueOnce(
      new Promise<ChatgptPlanStatusView>((_, reject) => {
        rejectSignIn = reject;
      }),
    );
    mocks.chatgptPlanSignInCancel.mockResolvedValueOnce(undefined);
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status).toEqual(chatgptSignedOut));

    let signing!: Promise<boolean>;
    await act(async () => {
      signing = result.current.signIn();
    });
    await act(async () => {
      await result.current.cancelSignIn();
    });
    await act(async () => {
      rejectSignIn({ version: 1, code: "sign_in_failed" });
      await signing;
    });

    expect(result.current.error).toBeNull();
    expect(result.current.signingIn).toBe(false);
    // One read on mount, and one after the cancelled sign-in ended.
    expect(mocks.chatgptPlanStatus).toHaveBeenCalledTimes(2);
  });

  it("reads the status back after a cancel when this hook has no sign-in in flight", async () => {
    mocks.chatgptPlanStatus
      .mockResolvedValueOnce({ ...chatgptSignedOut, state: "signing_in" })
      .mockResolvedValueOnce(chatgptSignedOut);
    mocks.chatgptPlanSignInCancel.mockResolvedValueOnce(undefined);
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status?.state).toBe("signing_in"));

    await act(async () => {
      await result.current.cancelSignIn();
    });

    expect(mocks.chatgptPlanSignInCancel).toHaveBeenCalledOnce();
    await waitFor(() => expect(result.current.status?.state).toBe("signed_out"));
  });

  it("stores the signed-out view that sign-out returns", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    mocks.chatgptPlanSignOut.mockResolvedValueOnce(chatgptSignedOut);
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status).toEqual(chatgptReady));

    let saved = false;
    await act(async () => {
      saved = await result.current.signOut();
    });

    expect(saved).toBe(true);
    expect(result.current.status).toEqual(chatgptSignedOut);
    expect(result.current.busy).toBe(false);
  });

  it("clears the first-sign-in notice from the status that acknowledgement returns", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    mocks.chatgptPlanWelcomeAck.mockResolvedValueOnce({ ...chatgptReady, welcome_pending: false });
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status?.welcome_pending).toBe(true));

    await act(async () => {
      await result.current.acknowledgeWelcome();
    });

    expect(mocks.chatgptPlanWelcomeAck).toHaveBeenCalledOnce();
    expect(result.current.status?.welcome_pending).toBe(false);
  });

  it("keeps the notice pending and reports the failure when acknowledgement fails", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    mocks.chatgptPlanWelcomeAck.mockRejectedValueOnce({ version: 1, code: "unexpected" });
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status?.welcome_pending).toBe(true));

    await act(async () => {
      await result.current.acknowledgeWelcome();
    });

    expect(result.current.status?.welcome_pending).toBe(true);
    expect(result.current.error).toBe("Could not save that choice. Try again.");
    expect(result.current.busy).toBe(false);
  });

  it("still reports a sign-in that settles after unmount, and writes nothing", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptSignedOut);
    let finishSignIn!: (value: ChatgptPlanStatusView) => void;
    mocks.chatgptPlanSignIn.mockReturnValueOnce(
      new Promise<ChatgptPlanStatusView>((resolve) => {
        finishSignIn = resolve;
      }),
    );
    const { result, unmount } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status).toEqual(chatgptSignedOut));

    let pending!: Promise<boolean>;
    await act(async () => {
      pending = result.current.signIn();
    });
    unmount();
    finishSignIn(chatgptReady);

    await expect(pending).resolves.toBe(true);
    expect(result.current.status).toEqual(chatgptSignedOut);
  });

  it("does not count a sign-in that comes back ready with an error as a change", async () => {
    mocks.chatgptPlanStatus.mockResolvedValue(chatgptSignedOut);
    mocks.chatgptPlanSignIn.mockResolvedValueOnce({ ...chatgptReady, error: "Sign-in timed out. Try again." });
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status).toEqual(chatgptSignedOut));

    let changed = true;
    await act(async () => {
      changed = await result.current.signIn();
    });

    expect(changed).toBe(false);
    expect(result.current.status?.state).toBe("ready");
    expect(result.current.status?.error).toBe("Sign-in timed out. Try again.");
  });

  it("does not count a sign-in that resolves signed out, such as a cancelled one, as a change", async () => {
    mocks.chatgptPlanStatus.mockResolvedValue(chatgptSignedOut);
    mocks.chatgptPlanSignIn.mockResolvedValueOnce(chatgptSignedOut);
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status).toEqual(chatgptSignedOut));

    let changed = true;
    await act(async () => {
      changed = await result.current.signIn();
    });

    expect(changed).toBe(false);
  });

  it("counts a sign-out as a change even when OpenAI did not confirm the revocation", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    mocks.chatgptPlanSignOut.mockResolvedValueOnce({ ...chatgptSignedOut, error: SIGN_OUT_UNCONFIRMED });
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status).toEqual(chatgptReady));

    let changed = false;
    await act(async () => {
      changed = await result.current.signOut();
    });

    expect(changed).toBe(true);
    expect(result.current.status?.state).toBe("signed_out");
    expect(result.current.status?.error).toBe(SIGN_OUT_UNCONFIRMED);
  });

  it("does not count a sign-out that leaves the account signed in as a change", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    mocks.chatgptPlanSignOut.mockResolvedValueOnce({ ...chatgptReady, welcome_pending: false });
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status).toEqual(chatgptReady));

    let changed = true;
    await act(async () => {
      changed = await result.current.signOut();
    });

    expect(changed).toBe(false);
  });

  it("counts the first-sign-in acknowledgement as saved when it carries no error", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    mocks.chatgptPlanWelcomeAck.mockResolvedValueOnce({ ...chatgptReady, welcome_pending: false });
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status?.welcome_pending).toBe(true));

    let saved = false;
    await act(async () => {
      saved = await result.current.acknowledgeWelcome();
    });

    expect(saved).toBe(true);
    expect(result.current.status?.welcome_pending).toBe(false);
  });

  it("does not count an acknowledgement that carries an error as saved", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    mocks.chatgptPlanWelcomeAck.mockResolvedValueOnce({
      ...chatgptReady,
      error: "Could not save the sign-in on this device.",
    });
    const { result } = renderHook(() => useChatgptPlan());
    await waitFor(() => expect(result.current.status?.welcome_pending).toBe(true));

    let saved = true;
    await act(async () => {
      saved = await result.current.acknowledgeWelcome();
    });

    expect(saved).toBe(false);
    expect(result.current.status?.welcome_pending).toBe(true);
    expect(result.current.status?.error).toBe("Could not save the sign-in on this device.");
  });
});
