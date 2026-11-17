import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatgptPlanStatusView, ClaudeEffort, ClaudeRuntimeStatusView } from "../../lib/generated/contracts";
import { Toaster } from "../../ui/Toaster";
import { resetToastsForTests } from "../../ui/toastStore";
import { PlanProvidersSection } from "./PlanProvidersSection";
import { SettingsGroup } from "./SettingsLayout";
import { CLAUDE_RUNTIME_COPY } from "./settingsCopy";

const mocks = vi.hoisted(() => ({
  claudeRuntimeStatus: vi.fn(),
  claudeRuntimeSetEnabled: vi.fn(),
  claudeRuntimeSetEffort: vi.fn(),
  chatgptPlanStatus: vi.fn(),
  chatgptPlanSignIn: vi.fn(),
  chatgptPlanSignInCancel: vi.fn(),
  chatgptPlanSignOut: vi.fn(),
  chatgptPlanWelcomeAck: vi.fn(),
  writeClipboardText: vi.fn(),
  invalidateModelCatalog: vi.fn(),
}));

vi.mock("../../lib/native", () => ({
  claudeRuntimeStatus: mocks.claudeRuntimeStatus,
  claudeRuntimeSetEnabled: mocks.claudeRuntimeSetEnabled,
  claudeRuntimeSetEffort: mocks.claudeRuntimeSetEffort,
  chatgptPlanStatus: mocks.chatgptPlanStatus,
  chatgptPlanSignIn: mocks.chatgptPlanSignIn,
  chatgptPlanSignInCancel: mocks.chatgptPlanSignInCancel,
  chatgptPlanSignOut: mocks.chatgptPlanSignOut,
  chatgptPlanWelcomeAck: mocks.chatgptPlanWelcomeAck,
}));

vi.mock("../chat/clipboard", () => ({
  writeClipboardText: mocks.writeClipboardText,
}));

vi.mock("../models/useModelCatalog", () => ({
  invalidateModelCatalog: mocks.invalidateModelCatalog,
}));

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
  account_label: "Work account",
  model_count: 2,
};

// The text the backend sends after a sign-out that OpenAI did not confirm (src-tauri plan_providers/service.rs).
const SIGN_OUT_UNCONFIRMED =
  "Signed out here, but OpenAI did not confirm it. You can also disconnect xFalcon in your ChatGPT settings.";

function renderSection(onWorkspaceChanged = vi.fn()) {
  // Mounted where ModelsConnectionsSection mounts it: inside its settings group.
  render(
    <>
      <SettingsGroup aria-label="Model and connection settings">
        <PlanProvidersSection onWorkspaceChanged={onWorkspaceChanged} />
      </SettingsGroup>
      <Toaster />
    </>,
  );
  return { onWorkspaceChanged };
}

beforeEach(() => {
  mocks.claudeRuntimeStatus.mockResolvedValue(claudeOff);
  mocks.chatgptPlanStatus.mockResolvedValue(chatgptSignedOut);
});

afterEach(() => {
  cleanup();
  resetToastsForTests();
  for (const mock of Object.values(mocks)) mock.mockReset();
});

describe("PlanProvidersSection: Claude card", () => {
  it("keeps Claude off with one description and no state until it is turned on", async () => {
    renderSection();

    const toggle = await screen.findByRole("switch", { name: "Use Claude on this device" });
    expect(toggle).not.toBeChecked();
    expect(screen.getByRole("heading", { name: "Claude (your Claude account)" })).toBeInTheDocument();
    expect(
      screen.getByText(
        "Runs the Claude Code program installed on this computer, under your own Claude plan. xFalcon is not affiliated with Anthropic.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/^Ready\./)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check again" })).not.toBeInTheDocument();
  });

  it("opens the consent notice when turned on, and Cancel keeps it off without any command", async () => {
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();
    const toggle = await screen.findByRole("switch", { name: "Use Claude on this device" });

    await user.click(toggle);

    const dialog = screen.getByRole("alertdialog", { name: "Use Claude on this device?" });
    expect(dialog).toHaveTextContent("The conversation and tool results are sent to Anthropic under your own plan.");
    expect(dialog).toHaveTextContent("Pro and Max content falls under Anthropic's Consumer Terms.");
    // No training or opt-out claim: none is sourced, and consent copy needs counsel review.
    expect(dialog).not.toHaveTextContent(/training|opt out/i);
    expect(dialog).toHaveTextContent("Team, Enterprise and API use falls under Anthropic's Commercial Terms.");
    expect(dialog).toHaveTextContent("xFalcon never sees or stores your Claude sign-in.");
    expect(dialog).toHaveTextContent("Anthropic may limit or suspend accounts, including for use in third-party apps.");
    expect(dialog).toHaveTextContent("xFalcon is not affiliated with Anthropic.");

    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(mocks.claudeRuntimeSetEnabled).not.toHaveBeenCalled();
    expect(toggle).not.toBeChecked();
    expect(onWorkspaceChanged).not.toHaveBeenCalled();
  });

  it("calls the enable command after Enable is pressed, then reports the change", async () => {
    mocks.claudeRuntimeSetEnabled.mockResolvedValueOnce(claudeReady);
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("switch", { name: "Use Claude on this device" }));

    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Enable" }));

    await waitFor(() => expect(mocks.claudeRuntimeSetEnabled).toHaveBeenCalledWith({ enabled: true }));
    await waitFor(() => expect(onWorkspaceChanged).toHaveBeenCalledOnce());
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(await screen.findByRole("switch", { name: "Use Claude on this device" })).toBeChecked();
    expect(screen.getByText("Ready. Plan: Max. Sign-in method: Claude subscription.")).toBeInTheDocument();
  });

  it("turns Claude off without a notice and reports the change", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(claudeReady);
    mocks.claudeRuntimeSetEnabled.mockResolvedValueOnce(claudeOff);
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();
    const toggle = await screen.findByRole("switch", { name: "Use Claude on this device" });
    expect(toggle).toBeChecked();

    await user.click(toggle);

    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    await waitFor(() => expect(mocks.claudeRuntimeSetEnabled).toHaveBeenCalledWith({ enabled: false }));
    await waitFor(() => expect(onWorkspaceChanged).toHaveBeenCalledOnce());
    await waitFor(() => expect(toggle).not.toBeChecked());
  });

  it.each([
    {
      name: "not installed",
      status: {
        ...claudeReady,
        state: "not_installed" as const,
        installed_version: null,
        plan: null,
        sign_in_method: null,
      },
      text: "Claude Code was not found on this computer. Install it, then choose Check again.",
    },
    {
      name: "outdated",
      status: {
        ...claudeReady,
        state: "outdated" as const,
        installed_version: "2.0.9",
        plan: null,
        sign_in_method: null,
      },
      text: "Claude Code 2.0.9 is older than the supported 2.1.295. Update it, then choose Check again.",
    },
    {
      name: "check failed",
      status: { ...claudeReady, state: "check_failed" as const, plan: null, sign_in_method: null },
      text: "Could not check Claude Code. Try again.",
    },
  ])("explains the $name state and offers Check again", async ({ status, text }) => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce({ ...status, enabled: true });
    renderSection();

    expect(await screen.findByText(text)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Check again" })).toBeInTheDocument();
  });

  it("re-checks when Check again is pressed and shows the new state", async () => {
    mocks.claudeRuntimeStatus
      .mockResolvedValueOnce({ ...claudeReady, state: "check_failed" as const, plan: null, sign_in_method: null })
      .mockResolvedValueOnce(claudeReady);
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Check again" }));

    expect(await screen.findByText("Ready. Plan: Max. Sign-in method: Claude subscription.")).toBeInTheDocument();
    expect(mocks.claudeRuntimeStatus).toHaveBeenCalledTimes(2);
    expect(onWorkspaceChanged).toHaveBeenCalledOnce();
  });

  it("shows the sign-in command in a code box and copies it on request", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce({
      ...claudeReady,
      state: "signed_out" as const,
      plan: null,
      sign_in_method: null,
    });
    mocks.writeClipboardText.mockResolvedValueOnce(undefined);
    renderSection();
    const user = userEvent.setup();

    expect(
      await screen.findByText("Not signed in. Run this command in a terminal, then choose Check again:"),
    ).toBeInTheDocument();
    expect(screen.getByText("claude auth login", { selector: "code" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /sign in/i })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Copy" }));

    expect(mocks.writeClipboardText).toHaveBeenCalledWith("claude auth login");
    expect(await screen.findByText("Copied the sign-in command.")).toBeInTheDocument();
  });

  it("reports the plan and sign-in method when Claude is ready", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(claudeReady);
    renderSection();

    expect(await screen.findByText("Ready. Plan: Max. Sign-in method: Claude subscription.")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Use Claude on this device" })).toBeChecked();
  });

  it("offers Check again after a failed status read rather than showing an empty card", async () => {
    mocks.claudeRuntimeStatus
      .mockRejectedValueOnce({ version: 1, code: "unexpected" })
      .mockResolvedValueOnce(claudeOff);
    const user = userEvent.setup();
    renderSection();

    expect(await screen.findByRole("alert")).toHaveTextContent("Could not check Claude Code. Try again.");
    expect(screen.getByRole("switch", { name: "Use Claude on this device" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Check again" }));

    await waitFor(() => expect(screen.getByRole("switch", { name: "Use Claude on this device" })).toBeEnabled());
    expect(mocks.claudeRuntimeStatus).toHaveBeenCalledTimes(2);
  });

  it("invalidates the model catalog once when Claude is turned on", async () => {
    mocks.claudeRuntimeSetEnabled.mockResolvedValueOnce(claudeReady);
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("switch", { name: "Use Claude on this device" }));

    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Enable" }));

    await waitFor(() => expect(onWorkspaceChanged).toHaveBeenCalledOnce());
    expect(mocks.invalidateModelCatalog).toHaveBeenCalledOnce();
  });

  it("invalidates the model catalog once when Claude is turned off", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(claudeReady);
    mocks.claudeRuntimeSetEnabled.mockResolvedValueOnce(claudeOff);
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("switch", { name: "Use Claude on this device" }));

    await waitFor(() => expect(onWorkspaceChanged).toHaveBeenCalledOnce());
    expect(mocks.invalidateModelCatalog).toHaveBeenCalledOnce();
  });

  it("invalidates the model catalog once when Claude is checked again", async () => {
    mocks.claudeRuntimeStatus
      .mockResolvedValueOnce({ ...claudeReady, state: "check_failed" as const, plan: null, sign_in_method: null })
      .mockResolvedValueOnce(claudeReady);
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Check again" }));

    await waitFor(() => expect(onWorkspaceChanged).toHaveBeenCalledOnce());
    expect(mocks.invalidateModelCatalog).toHaveBeenCalledOnce();
  });

  // These start from a switch that holds no focus, as a click on WKWebView leaves it. fireEvent does not focus the switch.
  it("moves focus to the switch once Enable completes", async () => {
    mocks.claudeRuntimeSetEnabled.mockResolvedValueOnce(claudeReady);
    renderSection();
    const user = userEvent.setup();
    const toggle = await screen.findByRole("switch", { name: "Use Claude on this device" });
    await waitFor(() => expect(toggle).toBeEnabled());
    fireEvent.click(toggle);

    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Enable" }));

    await waitFor(() => expect(toggle).toHaveFocus());
  });

  it("moves focus to the switch once it is turned off", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(claudeReady);
    mocks.claudeRuntimeSetEnabled.mockResolvedValueOnce(claudeOff);
    renderSection();
    const toggle = await screen.findByRole("switch", { name: "Use Claude on this device" });
    await waitFor(() => expect(toggle).toBeEnabled());

    fireEvent.click(toggle);

    await waitFor(() => expect(mocks.claudeRuntimeSetEnabled).toHaveBeenCalledWith({ enabled: false }));
    await waitFor(() => expect(toggle).toHaveFocus());
  });

  it("returns focus to the switch when the consent notice is cancelled", async () => {
    renderSection();
    const user = userEvent.setup();
    const toggle = await screen.findByRole("switch", { name: "Use Claude on this device" });
    await waitFor(() => expect(toggle).toBeEnabled());
    fireEvent.click(toggle);

    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Cancel" }));

    await waitFor(() => expect(toggle).toHaveFocus());
  });

  it("announces the loading lines as status while the first checks run", () => {
    mocks.claudeRuntimeStatus.mockReturnValueOnce(new Promise(() => {}));
    mocks.chatgptPlanStatus.mockReturnValueOnce(new Promise(() => {}));
    renderSection();

    expect(screen.getByText("Checking Claude Code…").parentElement).toHaveAttribute("role", "status");
    expect(screen.getByText("Checking your ChatGPT plan…").parentElement).toHaveAttribute("role", "status");
  });

  it("does not move focus on the initial load", async () => {
    renderSection();

    await waitFor(() => expect(screen.getByRole("switch", { name: "Use Claude on this device" })).toBeEnabled());
    await screen.findByRole("button", { name: "Continue with ChatGPT" });
    expect(document.body).toHaveFocus();
  });
});

// A ready Claude card with the given reasoning level stored.
function readyWithEffort(effort: ClaudeEffort): ClaudeRuntimeStatusView {
  return { ...claudeReady, effort };
}

const EFFORT_LABELS = ["Low", "Medium", "High (recommended)", "Extra high (xhigh)", "Max"];
const HIGH_HINT =
  "For work where verification matters: it checks its work and edge cases before answering, and it is the default here. " +
  "New chats start at this level. A chat keeps the level it started with. Higher levels think longer and use more of your Claude plan.";

describe("PlanProvidersSection: Claude reasoning effort", () => {
  it("is absent while Claude is off", async () => {
    renderSection();

    await screen.findByRole("switch", { name: "Use Claude on this device" });
    expect(screen.queryByText("Reasoning effort")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Reasoning effort" })).not.toBeInTheDocument();
  });

  it.each(["not_installed", "outdated", "signed_out", "ready", "check_failed"] as const)(
    "shows the stored level while Claude is on in the %s state",
    async (state) => {
      mocks.claudeRuntimeStatus.mockResolvedValueOnce({ ...readyWithEffort("low"), state });
      renderSection();

      expect(await screen.findByRole("button", { name: "Reasoning effort" })).toHaveTextContent("Low");
    },
  );

  it("shows the stored level as the selected choice", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(readyWithEffort("xhigh"));
    renderSection();

    expect(await screen.findByRole("button", { name: "Reasoning effort" })).toHaveTextContent("Extra high (xhigh)");
  });

  it("lists exactly the five levels in order", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(readyWithEffort("high"));
    renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Reasoning effort" }));

    expect(screen.getAllByRole("menuitemradio").map((option) => option.textContent)).toEqual(EFFORT_LABELS);
  });

  it("saves the chosen level, shows it, and does not report a model catalog change", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(readyWithEffort("high"));
    mocks.claudeRuntimeSetEffort.mockResolvedValueOnce(readyWithEffort("max"));
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();
    const trigger = await screen.findByRole("button", { name: "Reasoning effort" });

    await user.click(trigger);
    await user.click(screen.getByRole("menuitemradio", { name: "Max" }));

    await waitFor(() => expect(mocks.claudeRuntimeSetEffort).toHaveBeenCalledWith({ effort: "max" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Reasoning effort" })).toHaveTextContent("Max"));
    expect(onWorkspaceChanged).not.toHaveBeenCalled();
    expect(mocks.invalidateModelCatalog).not.toHaveBeenCalled();
  });

  it("keeps the stored level and shows the save-failed text when the save is rejected", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(readyWithEffort("high"));
    mocks.claudeRuntimeSetEffort.mockRejectedValueOnce({ version: 1, code: "unexpected" });
    renderSection();
    const user = userEvent.setup();
    const trigger = await screen.findByRole("button", { name: "Reasoning effort" });

    await user.click(trigger);
    await user.click(screen.getByRole("menuitemradio", { name: "Max" }));

    expect(await screen.findByText("The Claude setting could not be saved. Try again.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reasoning effort" })).toHaveTextContent("High (recommended)");
  });

  it("disables the select while the save is in flight", async () => {
    let finishSave!: (value: ClaudeRuntimeStatusView) => void;
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(readyWithEffort("high"));
    mocks.claudeRuntimeSetEffort.mockReturnValueOnce(
      new Promise<ClaudeRuntimeStatusView>((resolve) => {
        finishSave = resolve;
      }),
    );
    renderSection();
    const user = userEvent.setup();
    const trigger = await screen.findByRole("button", { name: "Reasoning effort" });

    await user.click(trigger);
    await user.click(screen.getByRole("menuitemradio", { name: "Max" }));

    await waitFor(() => expect(trigger).toBeDisabled());
    await act(async () => finishSave(readyWithEffort("max")));
    await waitFor(() => expect(trigger).toBeEnabled());
    expect(trigger).toHaveTextContent("Max");
  });

  it("moves focus back to the level select once its save settles", async () => {
    let finishSave!: (value: ClaudeRuntimeStatusView) => void;
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(readyWithEffort("high"));
    mocks.claudeRuntimeSetEffort.mockReturnValueOnce(
      new Promise<ClaudeRuntimeStatusView>((resolve) => {
        finishSave = resolve;
      }),
    );
    renderSection();
    const user = userEvent.setup();
    const trigger = await screen.findByRole("button", { name: "Reasoning effort" });

    await user.click(trigger);
    await user.click(screen.getByRole("menuitemradio", { name: "Max" }));
    await waitFor(() => expect(trigger).toBeDisabled());
    await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument());
    const focus = vi.spyOn(trigger, "focus");
    await act(async () => finishSave(readyWithEffort("max")));

    // The select is disabled while the save runs, and a browser drops focus from it then, so the card puts it back.
    await waitFor(() => expect(focus).toHaveBeenCalled());
    expect(trigger).toHaveFocus();
  });

  it("describes the stored level in the row hint and changes it with the level", async () => {
    mocks.claudeRuntimeStatus.mockResolvedValueOnce(readyWithEffort("high"));
    mocks.claudeRuntimeSetEffort.mockResolvedValueOnce(readyWithEffort("low"));
    renderSection();
    const user = userEvent.setup();
    const trigger = await screen.findByRole("button", { name: "Reasoning effort" });

    expect(screen.getByText(HIGH_HINT)).toBeInTheDocument();
    expect(trigger).toHaveAccessibleDescription(HIGH_HINT);

    await user.click(trigger);
    await user.click(screen.getByRole("menuitemradio", { name: "Low" }));

    const lowHint =
      "Quick exchanges that you review closely. New chats start at this level. A chat keeps the level it started with. Higher levels think longer and use more of your Claude plan.";
    await waitFor(() => expect(screen.getByText(lowHint)).toBeInTheDocument());
    expect(screen.queryByText(HIGH_HINT)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reasoning effort" })).toHaveAccessibleDescription(lowHint);
  });

  it("lists one copy entry per flag level, in the order the select shows them", () => {
    expect(Object.keys(CLAUDE_RUNTIME_COPY.effortLevels)).toEqual(["low", "medium", "high", "xhigh", "max"]);
  });
});

describe("PlanProvidersSection: ChatGPT card", () => {
  it("uses OpenAI's required wording and states what the plan does and does not share", async () => {
    renderSection();

    expect(await screen.findByRole("heading", { name: "Use your ChatGPT plan" })).toBeInTheDocument();
    expect(screen.getByText(/Plan usage is separate from xFalcon's own charges\./)).toBeInTheDocument();
    expect(
      screen.getByText(/A Plus plan's five-hour limit is shared by every app that uses the plan\./),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue with ChatGPT" })).toBeInTheDocument();
  });

  it("starts sign-in from the button and shows the waiting state while the browser step runs", async () => {
    mocks.chatgptPlanSignIn.mockReturnValueOnce(new Promise(() => {}));
    renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Continue with ChatGPT" }));

    expect(mocks.chatgptPlanSignIn).toHaveBeenCalledOnce();
    expect(await screen.findByText("Waiting for you to finish in your browser…")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Continue with ChatGPT" })).not.toBeInTheDocument();
  });

  it("ends the waiting state when Cancel is pressed during sign-in", async () => {
    let finishSignIn!: (value: ChatgptPlanStatusView) => void;
    mocks.chatgptPlanSignIn.mockReturnValueOnce(
      new Promise<ChatgptPlanStatusView>((resolve) => {
        finishSignIn = resolve;
      }),
    );
    mocks.chatgptPlanSignInCancel.mockResolvedValueOnce(undefined);
    renderSection();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Continue with ChatGPT" }));
    await screen.findByText("Waiting for you to finish in your browser…");

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(mocks.chatgptPlanSignInCancel).toHaveBeenCalledOnce();

    await act(async () => {
      finishSignIn(chatgptSignedOut);
    });

    expect(await screen.findByRole("button", { name: "Continue with ChatGPT" })).toBeInTheDocument();
    expect(screen.queryByText("Waiting for you to finish in your browser…")).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it.each([
    {
      name: "plan usage is off",
      state: "plan_usage_disabled" as const,
      text: "You signed in, but plan usage is off. Sign in again and allow plan usage.",
    },
    {
      name: "the sign-in expired",
      state: "needs_sign_in" as const,
      text: "Your ChatGPT sign-in expired. Sign in again.",
    },
  ])("explains that $name and offers Continue with ChatGPT", async ({ state, text }) => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce({ ...chatgptSignedOut, state });
    renderSection();

    expect(await screen.findByText(text)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue with ChatGPT" })).toBeInTheDocument();
  });

  it("shows the signed-in account, the plan label, the usage link and the model count when ready", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    renderSection();

    expect(await screen.findByText("Signed in as Work account")).toBeInTheDocument();
    expect(screen.getByText("Using ChatGPT plan")).toBeInTheDocument();
    expect(screen.getByText("2 models")).toBeInTheDocument();
    const usage = screen.getByRole("link", { name: /Manage usage/ });
    expect(usage).toHaveAttribute("href", "https://chatgpt.com/settings/usage");
    expect(usage).toHaveAttribute("target", "_blank");
  });

  it("signs out through its command and reports the change", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    mocks.chatgptPlanSignOut.mockResolvedValueOnce(chatgptSignedOut);
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Sign out" }));

    expect(await screen.findByRole("button", { name: "Continue with ChatGPT" })).toBeInTheDocument();
    expect(mocks.chatgptPlanSignOut).toHaveBeenCalledOnce();
    expect(onWorkspaceChanged).toHaveBeenCalledOnce();
  });

  it("offers the reason and a retry when the status check fails", async () => {
    mocks.chatgptPlanStatus
      .mockResolvedValueOnce({
        ...chatgptSignedOut,
        state: "check_failed",
        error: "The ChatGPT plan could not be reached.",
      })
      .mockResolvedValueOnce(chatgptSignedOut);
    renderSection();
    const user = userEvent.setup();

    expect(await screen.findByText("The ChatGPT plan could not be reached.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again" }));

    expect(await screen.findByRole("button", { name: "Continue with ChatGPT" })).toBeInTheDocument();
  });

  it("shows the first-sign-in notice once, and only Got it closes it", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce({ ...chatgptReady, welcome_pending: true });
    mocks.chatgptPlanWelcomeAck.mockResolvedValueOnce({ ...chatgptReady, welcome_pending: false });
    renderSection();
    const user = userEvent.setup();

    const notice = await screen.findByRole("dialog", { name: "You're using your ChatGPT plan" });
    expect(notice).toHaveTextContent(
      "Chats that use a ChatGPT model run on your ChatGPT plan, separate from xFalcon's own charges.",
    );

    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog", { name: "You're using your ChatGPT plan" })).toBeInTheDocument();

    await user.click(within(notice).getByRole("button", { name: "Got it" }));

    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "You're using your ChatGPT plan" })).not.toBeInTheDocument(),
    );
    expect(mocks.chatgptPlanWelcomeAck).toHaveBeenCalledOnce();
  });

  it("does not show the first-sign-in notice once it was acknowledged", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    renderSection();

    expect(await screen.findByText("Using ChatGPT plan")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("keeps the notice open with the reason when Got it cannot be saved", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce({ ...chatgptReady, welcome_pending: true });
    mocks.chatgptPlanWelcomeAck.mockRejectedValueOnce({ version: 1, code: "unexpected" });
    renderSection();
    const user = userEvent.setup();
    const notice = await screen.findByRole("dialog", { name: "You're using your ChatGPT plan" });

    await user.click(within(notice).getByRole("button", { name: "Got it" }));

    expect(await within(notice).findByRole("alert")).toHaveTextContent("Could not save that choice. Try again.");
    expect(screen.getByRole("dialog", { name: "You're using your ChatGPT plan" })).toBeInTheDocument();
  });

  it("invalidates the model catalog once when ChatGPT sign-in completes", async () => {
    mocks.chatgptPlanSignIn.mockResolvedValueOnce(chatgptReady);
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Continue with ChatGPT" }));

    await waitFor(() => expect(onWorkspaceChanged).toHaveBeenCalledOnce());
    expect(mocks.invalidateModelCatalog).toHaveBeenCalledOnce();
  });

  it("invalidates the model catalog once when ChatGPT is signed out", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    mocks.chatgptPlanSignOut.mockResolvedValueOnce(chatgptSignedOut);
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Sign out" }));

    await waitFor(() => expect(onWorkspaceChanged).toHaveBeenCalledOnce());
    expect(mocks.invalidateModelCatalog).toHaveBeenCalledOnce();
  });

  it("invalidates the model catalog once when the ChatGPT status is checked again", async () => {
    mocks.chatgptPlanStatus
      .mockResolvedValueOnce({
        ...chatgptSignedOut,
        state: "check_failed",
        error: "The ChatGPT plan could not be reached.",
      })
      .mockResolvedValueOnce(chatgptSignedOut);
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Try again" }));

    await waitFor(() => expect(onWorkspaceChanged).toHaveBeenCalledOnce());
    expect(mocks.invalidateModelCatalog).toHaveBeenCalledOnce();
  });

  it("shows the sign-out notice when OpenAI did not confirm the revocation, and keeps the sign-in offer", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    mocks.chatgptPlanSignOut.mockResolvedValueOnce({ ...chatgptSignedOut, error: SIGN_OUT_UNCONFIRMED });
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Sign out" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(SIGN_OUT_UNCONFIRMED);
    expect(screen.getByRole("button", { name: "Continue with ChatGPT" })).toBeInTheDocument();
    await waitFor(() => expect(onWorkspaceChanged).toHaveBeenCalledOnce());
  });

  it("shows a failed sign-in as an alert while the account stays ready", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce({ ...chatgptReady, error: "Sign-in timed out. Try again." });
    renderSection();

    expect(await screen.findByRole("alert")).toHaveTextContent("Sign-in timed out. Try again.");
    expect(screen.getByText("Signed in as Work account")).toBeInTheDocument();
    expect(screen.getByText("Using ChatGPT plan")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Manage usage/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign out" })).toBeInTheDocument();
  });

  it("shows no alert for a ready account without an error", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    renderSection();

    expect(await screen.findByText("Using ChatGPT plan")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("shows a failed check's reason once, in the alert, with the retry", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce({
      ...chatgptSignedOut,
      state: "check_failed",
      error: "The ChatGPT plan could not be reached.",
    });
    renderSection();

    expect(await screen.findByRole("alert")).toHaveTextContent("The ChatGPT plan could not be reached.");
    expect(screen.getAllByText("The ChatGPT plan could not be reached.")).toHaveLength(1);
    expect(screen.queryByText("Could not check your ChatGPT plan. Try again.")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("shows the generic check message when a failed check gives no reason", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce({ ...chatgptSignedOut, state: "check_failed" });
    renderSection();

    expect(await screen.findByText("Could not check your ChatGPT plan. Try again.")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("shows a status error inside the first-sign-in notice, and only there", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce({
      ...chatgptReady,
      welcome_pending: true,
      error: "Could not save the sign-in on this device.",
    });
    renderSection();

    const notice = await screen.findByRole("dialog", { name: "You're using your ChatGPT plan" });
    expect(within(notice).getByRole("alert")).toHaveTextContent("Could not save the sign-in on this device.");
    expect(screen.getAllByRole("alert")).toHaveLength(1);
  });

  it("does not report a sign-in that comes back with an error as a workspace change", async () => {
    mocks.chatgptPlanSignIn.mockResolvedValueOnce({ ...chatgptReady, error: "Sign-in timed out. Try again." });
    const { onWorkspaceChanged } = renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Continue with ChatGPT" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Sign-in timed out. Try again.");
    expect(onWorkspaceChanged).not.toHaveBeenCalled();
    expect(mocks.invalidateModelCatalog).not.toHaveBeenCalled();
  });

  it("moves focus to Sign out once a sign-in completes", async () => {
    mocks.chatgptPlanSignIn.mockResolvedValueOnce(chatgptReady);
    renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Continue with ChatGPT" }));

    await waitFor(() => expect(screen.getByRole("button", { name: "Sign out" })).toHaveFocus());
  });

  it("moves focus to Continue with ChatGPT once a sign-in fails", async () => {
    mocks.chatgptPlanSignIn.mockRejectedValueOnce({ version: 1, code: "unexpected" });
    renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Continue with ChatGPT" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Sign-in did not finish. Try again.");
    await waitFor(() => expect(screen.getByRole("button", { name: "Continue with ChatGPT" })).toHaveFocus());
  });

  it("leaves focus where the reader moved it while a sign-in was running", async () => {
    let finishSignIn!: (value: ChatgptPlanStatusView) => void;
    mocks.chatgptPlanSignIn.mockReturnValueOnce(
      new Promise<ChatgptPlanStatusView>((resolve) => {
        finishSignIn = resolve;
      }),
    );
    renderSection();
    const user = userEvent.setup();
    const elsewhere = document.createElement("input");
    document.body.appendChild(elsewhere);

    try {
      await user.click(await screen.findByRole("button", { name: "Continue with ChatGPT" }));
      await screen.findByText("Waiting for you to finish in your browser…");
      elsewhere.focus();

      await act(async () => {
        finishSignIn(chatgptReady);
      });

      await waitFor(() => expect(screen.getByRole("button", { name: "Sign out" })).toBeInTheDocument());
      expect(elsewhere).toHaveFocus();
    } finally {
      elsewhere.remove();
    }
  });

  it("moves focus to Continue with ChatGPT once sign-out completes", async () => {
    mocks.chatgptPlanStatus.mockResolvedValueOnce(chatgptReady);
    mocks.chatgptPlanSignOut.mockResolvedValueOnce(chatgptSignedOut);
    renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Sign out" }));

    await waitFor(() => expect(screen.getByRole("button", { name: "Continue with ChatGPT" })).toHaveFocus());
  });

  it("keeps focus off the card while the first-sign-in notice is open, then moves it after Got it", async () => {
    mocks.chatgptPlanSignIn.mockResolvedValueOnce({ ...chatgptReady, welcome_pending: true });
    mocks.chatgptPlanWelcomeAck.mockResolvedValueOnce({ ...chatgptReady, welcome_pending: false });
    renderSection();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: "Continue with ChatGPT" }));
    const notice = await screen.findByRole("dialog", { name: "You're using your ChatGPT plan" });
    expect(screen.getByRole("button", { name: "Sign out" })).not.toHaveFocus();

    await user.click(within(notice).getByRole("button", { name: "Got it" }));

    await waitFor(() => expect(screen.getByRole("button", { name: "Sign out" })).toHaveFocus());
  });
});
