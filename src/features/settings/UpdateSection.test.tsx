import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { UpdateSection } from "./UpdateSection";
import { resetPlatformForTests, setPlatform } from "../../lib/platform";
import { SettingsUpdateBadge } from "./SettingsUpdateBadge";
import { UpdateNotice } from "./UpdateNotice";
import {
  checkForUpdates,
  dismissUpdateNotice,
  getUpdateSnapshot,
  restartApp,
  setUpdateStoreForTests,
  startBackgroundUpdateChecks,
  stopBackgroundUpdateChecks,
  subscribeUpdate,
  type UpdateStoreState,
} from "./updateStore";
import { resetChatStore, setChatStoreForTests } from "../chat/chatStore";
import { getToasts, resetToastsForTests } from "../../ui/toastStore";

const checkMock = vi.hoisted(() => vi.fn());
const bootstrapMock = vi.hoisted(() => vi.fn());
const installMock = vi.hoisted(() => vi.fn());
const restartMock = vi.hoisted(() => vi.fn());
const progressListenMock = vi.hoisted(() => vi.fn());
const listenMock = vi.hoisted(() => vi.fn());
const previewRegisterMock = vi.hoisted(() => vi.fn());
const writeClipboardMock = vi.hoisted(() => vi.fn<(text: string) => Promise<void>>());

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

vi.mock("../../lib/native", () => ({
  appBootstrap: bootstrapMock,
  updateCheck: checkMock,
  updateInstall: installMock,
  updateRestart: restartMock,
  onUpdateDownloadProgress: progressListenMock,
  previewDocumentRegister: previewRegisterMock,
}));

vi.mock("../chat/clipboard", () => ({
  writeClipboardText: writeClipboardMock,
}));

describe("UpdateSection", () => {
  beforeEach(() => {
    bootstrapMock.mockResolvedValue({ version: 1, application_version: "0.5.0", platform: "windows", session: null });
    setUpdateStoreForTests({
      currentVersion: "0.5.0",
      status: "idle",
      availableVersion: null,
      releaseNotes: null,
      lastCheckTime: null,
      lastCheckSucceeded: null,
      errorMessage: null,
      downloadProgress: 0,
      downloadedBytes: 0,
      totalBytes: null,
      noticeDismissed: false,
      restarting: false,
    });
    resetChatStore();
    resetPlatformForTests();
    checkMock.mockReset();
    installMock.mockReset();
    restartMock.mockReset();
    restartMock.mockResolvedValue(undefined);
    progressListenMock.mockReset();
    progressListenMock.mockResolvedValue(vi.fn());
    listenMock.mockReset();
    listenMock.mockResolvedValue(vi.fn());
    previewRegisterMock.mockReset();
    writeClipboardMock.mockReset();
    writeClipboardMock.mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.unstubAllEnvs();
    stopBackgroundUpdateChecks();
    cleanup();
    setUpdateStoreForTests({ status: "idle" });
    vi.useRealTimers();
  });

  describe("version keepsake", () => {
    afterEach(resetToastsForTests);

    it("opens the thank-you dialog on the seventh tap and supports Escape and Close", async () => {
      const user = userEvent.setup();
      resetToastsForTests();
      render(<UpdateSection />);
      const version = screen.getByText("v0.5.0");
      for (let i = 0; i < 6; i += 1) await user.click(version);
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      await user.click(version);
      const dialog = screen.getByRole("dialog", { name: "Thank you for being here." });
      expect(dialog).toHaveTextContent("xFalcon Desktop v0.5.0");
      expect(dialog).toHaveTextContent("Built with care by the xFalcon team.");
      expect(dialog).not.toHaveTextContent("Thank you for using it.");
      expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
      expect(getToasts()).toHaveLength(0);

      const scrim = document.querySelector<HTMLDivElement>(".dialog-scrim");
      expect(scrim).not.toBeNull();
      for (let i = 0; i < 8; i += 1) {
        await user.click(scrim!);
      }
      expect(screen.getByRole("dialog", { name: "Thank you for being here." })).toBeInTheDocument();
      await user.keyboard("{Escape}");
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

      for (let i = 0; i < 7; i += 1) await user.click(version);
      await user.click(screen.getByRole("button", { name: "Close" }));
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });

    it("removes the thank-you dialog when Settings unmounts", async () => {
      const user = userEvent.setup();
      const { unmount } = render(<UpdateSection />);
      const version = screen.getByText("v0.5.0");
      for (let i = 0; i < 7; i += 1) await user.click(version);
      expect(screen.getByRole("dialog")).toBeInTheDocument();
      unmount();
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });

    it("gives no sign that the version can be clicked", () => {
      render(<UpdateSection />);
      const version = screen.getByText("v0.5.0");
      expect(version.getAttribute("role")).toBeNull();
      expect(version.getAttribute("tabindex")).toBeNull();
      expect(version.className).not.toMatch(/cursor|hover|underline/);
    });
  });

  it("keeps one version and update action visible after a failed check without build metadata", () => {
    const revision = "1234567890abcdef1234567890abcdef12345678";
    vi.stubEnv("VITE_APP_REVISION", revision);
    setUpdateStoreForTests({ status: "error", errorMessage: "Update check failed" });
    render(<UpdateSection />);
    expect(screen.getByRole("group", { name: "Software updates" })).toHaveAttribute("id", "settings-updates");
    expect(screen.getByText("v0.5.0")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Check for updates" })).toBeEnabled();
    expect(screen.queryByText(revision)).not.toBeInTheDocument();
    expect(screen.queryByText("Revision")).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: /Current build|Software updates/ })).not.toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Update check failed");
  });

  it("automatically downloads during a silent check without restarting", async () => {
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
    });

    const observed: string[] = [];
    const unsubscribe = subscribeUpdate(() => observed.push(getUpdateSnapshot().status));
    await checkForUpdates(true);
    unsubscribe();

    expect(observed).toEqual(["checking", "installing", "ready_to_restart"]);
    expect(installMock).toHaveBeenCalledTimes(1);
    expect(restartMock).not.toHaveBeenCalled();
    expect(getUpdateSnapshot()).toMatchObject({ status: "ready_to_restart", availableVersion: "0.6.0" });
  });

  it("checks at startup and on the native cadence event", async () => {
    let scheduledCheck: (() => void) | undefined;
    const unlisten = vi.fn();
    listenMock.mockImplementationOnce(async (event: string, listener: () => void) => {
      expect(event).toBe("xfalcon://update-check-requested");
      scheduledCheck = listener;
      return unlisten;
    });
    checkMock.mockResolvedValue({ version: 1, current_version: "0.5.0", available_version: null, release_notes: null });

    const stop = await startBackgroundUpdateChecks();
    await waitFor(() => expect(checkMock).toHaveBeenCalledTimes(1));

    scheduledCheck?.();
    await waitFor(() => expect(checkMock).toHaveBeenCalledTimes(2));
    stop();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("still performs the startup check when event subscription fails", async () => {
    listenMock.mockRejectedValueOnce(new Error("event bridge unavailable"));
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: null,
      release_notes: null,
    });

    await expect(startBackgroundUpdateChecks()).resolves.toEqual(expect.any(Function));
    await waitFor(() => expect(checkMock).toHaveBeenCalledTimes(1));
    expect(progressListenMock).toHaveBeenCalledTimes(1);
    expect(getUpdateSnapshot()).toMatchObject({ status: "idle", lastCheckSucceeded: true });
  });

  it("downloads while a run is active without restarting", async () => {
    setChatStoreForTests({ activeRunId: "run-1", runState: "running" });
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
    });

    await checkForUpdates(false);

    expect(installMock).toHaveBeenCalledTimes(1);
    expect(restartMock).not.toHaveBeenCalled();
    expect(getUpdateSnapshot().status).toBe("ready_to_restart");
  });

  it("does not restart while a run is active", async () => {
    setUpdateStoreForTests({ status: "ready_to_restart", availableVersion: "0.6.0" });
    setChatStoreForTests({ activeRunId: "run-1", runState: "running" });

    await expect(restartApp()).resolves.toBe(false);
    expect(restartMock).not.toHaveBeenCalled();
    expect(getUpdateSnapshot().errorMessage).toMatch(/Finish the active run/);
  });

  it("shows inline success feedback for five seconds and preserves check history", async () => {
    vi.useFakeTimers();
    checkMock.mockResolvedValue({
      version: 1,
      current_version: "0.5.0",
      available_version: null,
      release_notes: null,
      is_staged: false,
    });
    const { container } = render(<UpdateSection />);
    const feedbackSlot = container.querySelector<HTMLDivElement>(".settings-update-feedback");
    expect(feedbackSlot).toBeInTheDocument();
    await act(async () => {
      await checkForUpdates();
    });
    const checkedAt = getUpdateSnapshot().lastCheckTime;
    const success = screen.getByRole("status");
    expect(success).toHaveTextContent("xFalcon is up to date.");
    expect(feedbackSlot).toContainElement(success);
    const actions = screen.getByRole("button", { name: "Check for updates" }).parentElement;
    expect(actions).not.toContainElement(success);
    expect(success).toHaveClass("settings-update-feedback__badge", "type-label");
    expect(success).not.toHaveClass("type-caption", "settings-group__state");
    act(() => {
      vi.advanceTimersByTime(4_999);
    });
    expect(screen.getByText("xFalcon is up to date.")).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(screen.queryByText("xFalcon is up to date.")).not.toBeInTheDocument();
    expect(container.querySelector<HTMLDivElement>(".settings-update-feedback")).toBe(feedbackSlot);
    expect(getUpdateSnapshot().lastCheckTime).toBe(checkedAt);
    expect(getUpdateSnapshot().lastCheckSucceeded).toBe(true);
  });

  it.each([
    ["installing", { status: "installing", availableVersion: "0.6.0", downloadProgress: 32 }],
    ["error", { status: "error", errorMessage: "Update check failed" }],
    ["ready_to_restart", { status: "ready_to_restart", availableVersion: "0.6.0" }],
  ] as [string, Partial<UpdateStoreState>][])(
    "does not let an old success timer dismiss a newer %s state",
    async (_status, newerState) => {
      vi.useFakeTimers();
      checkMock.mockResolvedValue({
        version: 1,
        current_version: "0.5.0",
        available_version: null,
        release_notes: null,
        is_staged: false,
      });
      render(<UpdateSection />);
      await act(async () => {
        await checkForUpdates();
      });
      act(() => {
        vi.advanceTimersByTime(4_999);
      });
      act(() => {
        setUpdateStoreForTests(newerState);
      });
      act(() => {
        vi.advanceTimersByTime(1);
      });
      expect(getUpdateSnapshot().status).toBe(newerState.status);
      if (newerState.status === "ready_to_restart") {
        expect(screen.getByRole("button", { name: "Restart app" })).toBeInTheDocument();
      }
      if (newerState.status === "installing") {
        expect(screen.getByRole("button", { name: "Downloading (32%)" })).toBeInTheDocument();
      }
      if (newerState.status === "error") {
        expect(screen.getByRole("alert")).toHaveTextContent("Update check failed");
      }
    },
  );

  it("resets the five-second feedback lifetime after a repeated successful check", async () => {
    vi.useFakeTimers();
    checkMock.mockResolvedValue({
      version: 1,
      current_version: "0.5.0",
      available_version: null,
      release_notes: null,
      is_staged: false,
    });
    render(<UpdateSection />);
    await act(async () => {
      await checkForUpdates();
    });
    act(() => {
      vi.advanceTimersByTime(3_000);
    });
    await act(async () => {
      await checkForUpdates();
    });
    act(() => {
      vi.advanceTimersByTime(4_999);
    });
    expect(screen.getByText("xFalcon is up to date.")).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(screen.queryByText("xFalcon is up to date.")).not.toBeInTheDocument();
  });

  it("renders current version and idle check button", () => {
    render(<UpdateSection />);
    expect(screen.getByText(/xFalcon Desktop/)).toBeInTheDocument();
    expect(screen.getByText(/v0.5.0/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Check for updates" })).toBeInTheDocument();
  });

  it("renders passive sanitized Markdown while installing and keeps the download action", () => {
    const notes = [
      "# What's new",
      "",
      "This is **important**.",
      "Inline code uses `release_version`.",
      "",
      "- First item",
      "- Second item",
      "",
      "```html",
      "<script>alert(1)</script>",
      "```",
      "",
      "[Release guide](https://example.com/updates)",
      "",
      "[Unsafe](javascript:alert(1))",
      "",
      "<img src=x onerror=alert(2)>",
      "",
      "![Remote image](https://example.com/image.png)",
      "",
      "| Area | Change |",
      "| --- | --- |",
      "| Reports | Faster |",
    ].join("\n");
    setUpdateStoreForTests({
      status: "installing",
      availableVersion: "0.6.0",
      releaseNotes: notes,
      downloadProgress: 42,
    });

    const { container } = render(<UpdateSection />);

    expect(screen.getByRole("heading", { name: "What's new" })).toBeInTheDocument();
    expect(screen.getByText("important")).toBeInTheDocument();
    expect(screen.getByText("First item")).toBeInTheDocument();
    expect(screen.getByText("Second item")).toBeInTheDocument();
    expect(screen.getByText("release_version")).toHaveClass("assistant-markdown__inline-code");
    expect(container.textContent).toContain("<script>alert(1)</script>");
    expect(container.querySelector("code.language-html")?.textContent).toContain("<script>alert(1)</script>");
    expect(screen.getByRole("link", { name: /Release guide/ })).toHaveAttribute("href", "https://example.com/updates");
    expect(screen.queryByRole("link", { name: "Unsafe" })).not.toBeInTheDocument();
    expect(screen.getByText("Image omitted: Remote image")).toBeInTheDocument();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("[onerror]")).toBeNull();
    expect(container.querySelector("pre.max-h-80.overflow-auto")).not.toBeNull();
    expect(screen.getByRole("region", { name: "Scrollable release notes table" })).toContainElement(
      screen.getByRole("table"),
    );
    expect(screen.getByRole("button", { name: "Downloading (42%)" })).toBeDisabled();
    expect(screen.getByText("Version 0.6.0 release notes")).toBeInTheDocument();
    expect(container.querySelector("iframe, script")).toBeNull();
    expect(previewRegisterMock).not.toHaveBeenCalled();
  });

  it("shows only the rendered release notes, without a Markdown header or source and copy actions", () => {
    const notes = "# Exact source\n\n- Keep **all** formatting markers.\n";
    setUpdateStoreForTests({ status: "ready_to_restart", availableVersion: "0.6.0", releaseNotes: notes });

    render(<UpdateSection />);

    expect(screen.getByRole("heading", { name: "Exact source" })).toBeInTheDocument();
    expect(screen.getByText("all", { selector: "strong" })).toBeInTheDocument();
    expect(screen.queryByText("Markdown", { exact: true })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Show source of Markdown" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Copy Markdown to clipboard" })).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Markdown source")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restart app" })).toBeEnabled();
  });

  it("hides the release notes when there are no notes or no staged update", () => {
    setUpdateStoreForTests({ status: "installing", availableVersion: "0.6.0", releaseNotes: null });
    const { rerender } = render(<UpdateSection />);
    expect(screen.queryByText("Version 0.6.0 release notes")).not.toBeInTheDocument();

    act(() => {
      setUpdateStoreForTests({ status: "idle", releaseNotes: "# Not an active update" });
    });
    rerender(<UpdateSection />);

    expect(screen.queryByRole("heading", { name: "Not an active update" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Check for updates" })).toBeInTheDocument();
  });

  it("triggers check for updates on button click and reports up to date", async () => {
    const user = userEvent.setup();
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: null,
      release_notes: null,
    });

    render(<UpdateSection />);
    const checkBtn = screen.getByRole("button", { name: "Check for updates" });
    await user.click(checkBtn);

    await waitFor(() => {
      expect(screen.getByText("xFalcon is up to date.")).toBeInTheDocument();
    });
  });

  it("automatically stages a manually discovered update without a download or decline action", async () => {
    const user = userEvent.setup();
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: "Fixes",
      is_staged: false,
    });
    render(<UpdateSection />);
    await user.click(screen.getByRole("button", { name: "Check for updates" }));
    expect(await screen.findByRole("button", { name: "Restart app" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Download|Decline/ })).not.toBeInTheDocument();
    const releaseNotes = screen.getByText("Fixes", { selector: "p" });
    expect(releaseNotes).toBeVisible();
    expect(installMock).toHaveBeenCalledTimes(1);
    expect(restartMock).not.toHaveBeenCalled();
  });

  it("reports automatic download progress and completion", async () => {
    let finishDownload: (() => void) | undefined;
    let progress:
      ((value: { version: 1; downloaded_bytes: number; total_bytes: number; percent: number }) => void) | undefined;
    progressListenMock.mockImplementationOnce(async (listener: typeof progress) => {
      progress = listener;
      return vi.fn();
    });
    installMock.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finishDownload = resolve;
        }),
    );
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
    });

    render(<UpdateSection />);
    await startBackgroundUpdateChecks();
    await waitFor(() => expect(installMock).toHaveBeenCalledTimes(1));
    progress?.({ version: 1, downloaded_bytes: 35, total_bytes: 100, percent: 35 });

    expect(await screen.findByRole("button", { name: "Downloading (35%)" })).toBeDisabled();
    expect(screen.getByRole("progressbar", { name: "Update download progress" })).toHaveAttribute(
      "aria-valuenow",
      "35",
    );

    finishDownload?.();
    await waitFor(() => expect(getUpdateSnapshot().status).toBe("ready_to_restart"));
    expect(restartMock).not.toHaveBeenCalled();
  });

  it("silent startup check failure stays silent and leaves no error alert", async () => {
    checkMock.mockRejectedValueOnce(new Error("Offline / DNS failure"));
    await checkForUpdates(true);

    const snapshot = getUpdateSnapshot();
    expect(snapshot.status).toBe("idle");
    expect(snapshot.errorMessage).toBeNull();
    expect(snapshot.lastCheckSucceeded).toBe(false);
    expect(snapshot.lastCheckTime).not.toBeNull();
  });

  it("renders error message on manual check failure", async () => {
    const user = userEvent.setup();
    checkMock.mockRejectedValueOnce(new Error("Network unreachable"));

    render(<UpdateSection />);
    await user.click(screen.getByRole("button", { name: "Check for updates" }));

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent("Update check failed: Network unreachable");
    });
  });

  it("keeps the install action and determinate progress while downloading", () => {
    setUpdateStoreForTests({
      status: "installing",
      availableVersion: "0.6.0",
      downloadProgress: 42,
    });

    render(<UpdateSection />);

    expect(screen.getByRole("button", { name: "Downloading (42%)" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Check for updates" })).not.toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "Update download progress" })).toHaveAttribute(
      "aria-valuenow",
      "42",
    );
  });

  it("uses indeterminate progress when the updater does not report a size", () => {
    setUpdateStoreForTests({
      status: "installing",
      availableVersion: "0.6.0",
      downloadProgress: null,
    });

    render(<UpdateSection />);

    expect(screen.getByRole("button", { name: "Downloading…" })).toBeDisabled();
    expect(screen.getByRole("progressbar", { name: "Update download progress" })).not.toHaveAttribute("aria-valuenow");
  });

  it("renders Restart App button when ready_to_restart and invokes native restart", async () => {
    const user = userEvent.setup();
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: true,
    });
    setUpdateStoreForTests({
      status: "ready_to_restart",
      availableVersion: "0.6.0",
      downloadProgress: 100,
    });

    render(<UpdateSection />);
    const restartBtn = screen.getByRole("button", { name: "Restart app" });
    expect(restartBtn).toBeInTheDocument();

    await user.click(restartBtn);
    await waitFor(() => expect(restartMock).toHaveBeenCalledTimes(1));
  });

  it("keeps restart available when the native relaunch fails", async () => {
    const user = userEvent.setup();
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: true,
    });
    restartMock.mockRejectedValueOnce(new Error("process refused"));
    setUpdateStoreForTests({
      status: "ready_to_restart",
      availableVersion: "0.6.0",
      downloadProgress: 100,
    });

    render(<UpdateSection />);
    await user.click(screen.getByRole("button", { name: "Restart app" }));

    expect(await screen.findByRole("status")).toHaveTextContent("Failed to restart application: process refused");
    expect(screen.getByRole("button", { name: "Restart app" })).toBeEnabled();
  });
  it("restores a verified staged update without another download", async () => {
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: true,
    });
    render(<UpdateNotice />);
    await checkForUpdates(true);
    expect(await screen.findByRole("button", { name: "Restart to update" })).toBeInTheDocument();
    expect(installMock).not.toHaveBeenCalled();
  });

  it("automatically downloads, lets the user defer restart, and retains the settings action", async () => {
    const user = userEvent.setup();
    checkMock.mockResolvedValue({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: false,
    });
    render(
      <>
        <UpdateNotice />
        <UpdateSection />
        <SettingsUpdateBadge onOpenUpdates={vi.fn()} />
      </>,
    );
    await act(async () => {
      await checkForUpdates();
    });
    expect(await screen.findByRole("button", { name: "Restart to update" })).toBeInTheDocument();
    expect(restartMock).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Later" }));
    expect(screen.queryByRole("region", { name: "Software update" })).not.toBeInTheDocument();
    expect(screen.getByText("1 update available")).toBeInTheDocument();
    checkMock.mockResolvedValue({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: true,
    });
    await user.click(screen.getByRole("button", { name: "Restart app" }));
    await waitFor(() => expect(restartMock).toHaveBeenCalledTimes(1));
  });

  it("reports a failed download in the app-wide notice and permits a fresh check", async () => {
    const user = userEvent.setup();
    checkMock.mockResolvedValue({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: false,
    });
    installMock.mockRejectedValueOnce(new Error("connection lost"));
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: false,
    });
    render(<UpdateNotice />);
    await act(async () => {
      await checkForUpdates();
    });
    expect(await screen.findByText("Update download failed: connection lost")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("button", { name: "Restart to update" })).toBeInTheDocument();
    expect(restartMock).not.toHaveBeenCalled();
  });

  it("prevents duplicate restart commands while applying", async () => {
    setUpdateStoreForTests({ status: "ready_to_restart", availableVersion: "0.6.0" });
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: true,
    });
    let finish: (() => void) | undefined;
    restartMock.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    const first = restartApp();
    await expect(restartApp()).resolves.toBe(false);
    await waitFor(() => expect(restartMock).toHaveBeenCalledTimes(1));
    finish?.();
    await expect(first).resolves.toBe(true);
    expect(restartMock).toHaveBeenCalledTimes(1);
  });

  it("can recheck after a staged file fails to apply and offer a fresh download", async () => {
    const user = userEvent.setup();
    setUpdateStoreForTests({ status: "ready_to_restart", availableVersion: "0.6.0" });
    restartMock.mockRejectedValueOnce(new Error("Staged update is unavailable"));
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: true,
    });
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: false,
    });
    render(<UpdateNotice />);
    await user.click(screen.getByRole("button", { name: "Restart to update" }));
    await user.click(await screen.findByRole("button", { name: "Check again" }));
    expect(await screen.findByRole("button", { name: "Restart to update" })).toBeInTheDocument();
  });

  it("retries a failed automatic download on the next scheduled event without a retry loop", async () => {
    let scheduledCheck: (() => void) | undefined;
    listenMock.mockImplementationOnce(async (_event: string, listener: () => void) => {
      scheduledCheck = listener;
      return vi.fn();
    });
    checkMock.mockResolvedValue({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: false,
    });
    installMock.mockRejectedValueOnce(new Error("offline"));
    await startBackgroundUpdateChecks();
    await waitFor(() => expect(getUpdateSnapshot().status).toBe("error"));
    expect(installMock).toHaveBeenCalledTimes(1);
    expect(checkMock).toHaveBeenCalledTimes(1);
    scheduledCheck?.();
    await waitFor(() => expect(getUpdateSnapshot().status).toBe("ready_to_restart"));
    expect(installMock).toHaveBeenCalledTimes(2);
    expect(restartMock).not.toHaveBeenCalled();
    // A staged update is still checked against the manifest. The same version is not downloaded again.
    checkMock.mockResolvedValue({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: true,
    });
    scheduledCheck?.();
    await waitFor(() => expect(checkMock).toHaveBeenCalledTimes(3));
    expect(installMock).toHaveBeenCalledTimes(2);
    expect(getUpdateSnapshot()).toMatchObject({ status: "ready_to_restart", availableVersion: "0.6.0" });
  });

  it("preserves dismissal when a silent refresh fails", async () => {
    dismissUpdateNotice();
    checkMock.mockRejectedValueOnce(new Error("offline"));
    await checkForUpdates(true);
    expect(getUpdateSnapshot()).toMatchObject({
      status: "idle",
      noticeDismissed: true,
      lastCheckSucceeded: false,
      errorMessage: null,
    });
  });

  it.each(["macos", "windows"] as const)(
    "shows replacement and administrator password guidance only on macOS (%s)",
    (platform) => {
      setPlatform(platform);
      setUpdateStoreForTests({ status: "ready_to_restart", availableVersion: "0.6.0" });
      render(
        <>
          <UpdateNotice />
          <UpdateSection />
        </>,
      );
      expect(
        screen.queryAllByText(
          /Updates require a writable Applications folder\. macOS may ask for an administrator password\./,
        ),
      ).toHaveLength(platform === "macos" ? 2 : 0);
    },
  );

  it("does not duplicate an automatic download when another check arrives", async () => {
    let finish: (() => void) | undefined;
    installMock.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: false,
    });
    const first = checkForUpdates(true);
    await waitFor(() => expect(getUpdateSnapshot().status).toBe("installing"));
    await checkForUpdates(true);
    expect(checkMock).toHaveBeenCalledTimes(1);
    expect(installMock).toHaveBeenCalledTimes(1);
    finish?.();
    await first;
    expect(getUpdateSnapshot().status).toBe("ready_to_restart");
    expect(restartMock).not.toHaveBeenCalled();
  });

  it("replaces a staged update when a scheduled check finds a newer release", async () => {
    setUpdateStoreForTests({ status: "ready_to_restart", availableVersion: "0.6.0", downloadProgress: 100 });
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.7.0",
      release_notes: null,
      is_staged: false,
    });
    const observed: string[] = [];
    const unsubscribe = subscribeUpdate(() => observed.push(getUpdateSnapshot().status));
    await checkForUpdates(true);
    unsubscribe();

    expect(observed).toEqual(["installing", "ready_to_restart"]);
    expect(installMock).toHaveBeenCalledTimes(1);
    expect(restartMock).not.toHaveBeenCalled();
    expect(getUpdateSnapshot()).toMatchObject({ status: "ready_to_restart", availableVersion: "0.7.0" });
  });

  it("keeps a staged update and its dismissal when a scheduled check names the same version", async () => {
    setUpdateStoreForTests({
      status: "ready_to_restart",
      availableVersion: "0.6.0",
      downloadProgress: 100,
      noticeDismissed: true,
    });
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.6.0",
      release_notes: null,
      is_staged: true,
    });
    const observed: string[] = [];
    const unsubscribe = subscribeUpdate(() => observed.push(getUpdateSnapshot().status));
    await checkForUpdates(true);
    unsubscribe();

    expect(observed).toEqual(["ready_to_restart"]);
    expect(installMock).not.toHaveBeenCalled();
    expect(getUpdateSnapshot()).toMatchObject({
      status: "ready_to_restart",
      availableVersion: "0.6.0",
      noticeDismissed: true,
    });
  });

  it("keeps a staged update on offer when a scheduled check fails", async () => {
    setUpdateStoreForTests({ status: "ready_to_restart", availableVersion: "0.6.0", downloadProgress: 100 });
    checkMock.mockRejectedValueOnce(new Error("offline"));
    await checkForUpdates(true);

    expect(getUpdateSnapshot()).toMatchObject({
      status: "ready_to_restart",
      availableVersion: "0.6.0",
      lastCheckSucceeded: false,
      errorMessage: null,
    });
  });

  it("leaves a restart error on screen instead of refreshing it in the background", async () => {
    setUpdateStoreForTests({
      status: "ready_to_restart",
      availableVersion: "0.6.0",
      errorMessage: "Failed to restart application: process refused",
    });
    await checkForUpdates(true);

    expect(checkMock).not.toHaveBeenCalled();
    expect(getUpdateSnapshot().errorMessage).toBe("Failed to restart application: process refused");
  });

  it("downloads a newer release instead of installing the staged one when Restart is pressed", async () => {
    setUpdateStoreForTests({ status: "ready_to_restart", availableVersion: "0.6.0", downloadProgress: 100 });
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.7.0",
      release_notes: null,
      is_staged: false,
    });

    await expect(restartApp()).resolves.toBe(false);
    expect(restartMock).not.toHaveBeenCalled();
    expect(installMock).toHaveBeenCalledTimes(1);
    expect(getUpdateSnapshot()).toMatchObject({
      status: "ready_to_restart",
      availableVersion: "0.7.0",
      restarting: false,
    });

    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.7.0",
      release_notes: null,
      is_staged: true,
    });
    await expect(restartApp()).resolves.toBe(true);
    expect(restartMock).toHaveBeenCalledTimes(1);
    expect(installMock).toHaveBeenCalledTimes(1);
  });

  it("moves the notice to the newer release when Restart finds one", async () => {
    const user = userEvent.setup();
    setUpdateStoreForTests({ status: "ready_to_restart", availableVersion: "0.6.0", downloadProgress: 100 });
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: "0.7.0",
      release_notes: null,
      is_staged: false,
    });
    render(<UpdateNotice />);

    await user.click(screen.getByRole("button", { name: "Restart to update" }));

    expect(await screen.findByText("Version 0.7.0 is ready to install")).toBeInTheDocument();
    expect(restartMock).not.toHaveBeenCalled();
  });

  it("applies the verified staged package when the manifest cannot be reached at restart", async () => {
    setUpdateStoreForTests({ status: "ready_to_restart", availableVersion: "0.6.0", downloadProgress: 100 });
    checkMock.mockRejectedValueOnce(new Error("offline"));

    await expect(restartApp()).resolves.toBe(true);
    expect(restartMock).toHaveBeenCalledTimes(1);
    expect(installMock).not.toHaveBeenCalled();
  });

  it("does not install when the channel no longer offers a newer version at restart", async () => {
    setUpdateStoreForTests({ status: "ready_to_restart", availableVersion: "0.6.0", downloadProgress: 100 });
    checkMock.mockResolvedValueOnce({
      version: 1,
      current_version: "0.5.0",
      available_version: null,
      release_notes: null,
      is_staged: false,
    });

    await expect(restartApp()).resolves.toBe(false);
    expect(restartMock).not.toHaveBeenCalled();
    expect(getUpdateSnapshot()).toMatchObject({ status: "up_to_date", availableVersion: null, restarting: false });
  });
});
