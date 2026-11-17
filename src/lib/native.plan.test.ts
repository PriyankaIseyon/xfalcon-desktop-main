import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  chatgptPlanSignIn,
  chatgptPlanSignInCancel,
  chatgptPlanSignOut,
  chatgptPlanStatus,
  chatgptPlanWelcomeAck,
  claudeRuntimeSetEffort,
  claudeRuntimeSetEnabled,
  claudeRuntimeStatus,
} from "./native";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: vi.fn(),
  invoke: invokeMock,
}));

beforeEach(() => invokeMock.mockReset());

describe("plan-provider native boundary", () => {
  it("reads the Claude runtime status with no arguments", async () => {
    invokeMock.mockResolvedValueOnce({});

    await claudeRuntimeStatus();

    expect(invokeMock).toHaveBeenCalledWith("claude_runtime_status");
  });

  it("sends the enable choice as one typed input", async () => {
    invokeMock.mockResolvedValueOnce({});

    await claudeRuntimeSetEnabled({ enabled: true });

    expect(invokeMock).toHaveBeenCalledWith("claude_runtime_set_enabled", { input: { enabled: true } });
  });

  it("resolves the Claude status view the command returns", async () => {
    const view = { version: 1, enabled: false, state: "disabled" };
    invokeMock.mockResolvedValueOnce(view);

    await expect(claudeRuntimeSetEnabled({ enabled: false })).resolves.toBe(view);
  });

  it("sends the reasoning level as one typed input", async () => {
    invokeMock.mockResolvedValueOnce({});

    await claudeRuntimeSetEffort({ effort: "max" });

    expect(invokeMock).toHaveBeenCalledWith("claude_runtime_set_effort", { input: { effort: "max" } });
  });

  it("resolves the Claude status view the effort command returns", async () => {
    const view = { version: 1, enabled: true, state: "ready", effort: "max" };
    invokeMock.mockResolvedValueOnce(view);

    await expect(claudeRuntimeSetEffort({ effort: "max" })).resolves.toBe(view);
  });

  it("reads the ChatGPT plan status with no arguments", async () => {
    invokeMock.mockResolvedValueOnce({});

    await chatgptPlanStatus();

    expect(invokeMock).toHaveBeenCalledWith("chatgpt_plan_status");
  });

  it("starts sign-in with no arguments and resolves with the view the command returns", async () => {
    const view = { version: 1, state: "ready" };
    invokeMock.mockResolvedValueOnce(view);

    await expect(chatgptPlanSignIn()).resolves.toBe(view);
    expect(invokeMock).toHaveBeenCalledWith("chatgpt_plan_sign_in");
  });

  it("cancels sign-in with no arguments", async () => {
    invokeMock.mockResolvedValueOnce(undefined);

    await chatgptPlanSignInCancel();

    expect(invokeMock).toHaveBeenCalledWith("chatgpt_plan_sign_in_cancel");
  });

  it("signs out through its own command", async () => {
    invokeMock.mockResolvedValueOnce({});

    await chatgptPlanSignOut();

    expect(invokeMock).toHaveBeenCalledWith("chatgpt_plan_sign_out");
  });

  it("acknowledges the first-sign-in notice through its own command", async () => {
    invokeMock.mockResolvedValueOnce({});

    await chatgptPlanWelcomeAck();

    expect(invokeMock).toHaveBeenCalledWith("chatgpt_plan_welcome_ack");
  });
});
