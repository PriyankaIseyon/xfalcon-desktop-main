import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ReportTools } from "./ReportTools";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { getToasts, dismissToast } from "../../ui/toastStore";
import { invoke } from "@tauri-apps/api/core";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
beforeEach(() => {
  onExport.mockClear();
  vi.mocked(invoke).mockClear();
  for (const t of getToasts()) dismissToast(t.id);
});
const onExport = vi.fn();
function mount(overrides: { exporting?: boolean; disabled?: boolean } = {}) {
  render(
    <header>
      <ReportTools
        title="Revenue report"
        disabled={overrides.disabled ?? false}
        exportLabel="an HTML file"
        exporting={overrides.exporting ?? false}
        onExport={onExport}
      />
    </header>,
  );
}
async function select(label: string, option: string) {
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: label }));
  await user.click(screen.getByRole("menuitemradio", { name: option }));
}

describe("report action prototypes", () => {
  it("keeps actions disabled until the real viewer is ready", () => {
    mount({ disabled: true });
    for (const button of screen.getAllByRole("button")) expect(button).toBeDisabled();
  });
  it("offers no demo refresh: Refresh is the viewer's real, server-side control", () => {
    mount();
    expect(screen.queryByRole("button", { name: "Refresh now" })).not.toBeInTheDocument();
  });
  it("shows the calendar, validates bounds and timezone, and previews a daily schedule", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(screen.getByRole("button", { name: "Schedule a refresh" }));
    expect(screen.getByRole("group", { name: "Start date calendar" })).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent("Every day at 09:00");
    fireEvent.change(screen.getByLabelText("Start date"), { target: { value: "2030-08-20" } });
    fireEvent.change(screen.getByLabelText("End date"), { target: { value: "2030-08-19" } });
    expect(screen.getByRole("button", { name: "Confirm schedule" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("End date"), { target: { value: "2030-08-21" } });
    fireEvent.change(screen.getByLabelText("Timezone"), { target: { value: "Fake/Zone" } });
    expect(screen.getByRole("button", { name: "Confirm schedule" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Timezone"), { target: { value: "UTC" } });
    await user.click(screen.getByRole("button", { name: "Confirm schedule" }));
    expect(screen.getByRole("status")).toHaveTextContent("No refresh was scheduled");
    expect(invoke).not.toHaveBeenCalled();
  });
  it("lets the calendar change months and select a date", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(screen.getByRole("button", { name: "Schedule a refresh" }));
    fireEvent.change(screen.getByLabelText("Start date"), { target: { value: "2030-08-20" } });
    await user.click(screen.getByRole("button", { name: "Next month" }));
    await user.click(screen.getByRole("button", { name: /September 10, 2030/ }));
    expect(screen.getByLabelText("Start date")).toHaveValue("2030-09-10");
  });
  it("requires a weekly day and displays selected weekdays", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(screen.getByRole("button", { name: "Schedule a refresh" }));
    await select("Frequency", "Weekly");
    await user.click(screen.getByRole("button", { name: "Monday" }));
    expect(screen.getByRole("button", { name: "Confirm schedule" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Friday" }));
    expect(screen.getByRole("status")).toHaveTextContent("Every Friday at 09:00");
    expect(screen.getByRole("button", { name: "Confirm schedule" })).toBeEnabled();
  });
  it("validates custom cron and describes it before confirmation", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(screen.getByRole("button", { name: "Schedule a refresh" }));
    await select("Frequency", "Custom cron");
    fireEvent.change(screen.getByLabelText("Cron expression"), { target: { value: "70 * * * *" } });
    expect(screen.getByRole("button", { name: "Confirm schedule" })).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent("Minute");
    fireEvent.change(screen.getByLabelText("Cron expression"), { target: { value: "0 9 * * 1-5" } });
    expect(screen.getByRole("status")).toHaveTextContent("At 09:00");
    expect(screen.getByRole("status")).toHaveTextContent("Friday");
    expect(screen.getByRole("button", { name: "Confirm schedule" })).toBeEnabled();
  });
  it("provides monthly and hourly controls without leaking hidden validation", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(screen.getByRole("button", { name: "Schedule a refresh" }));
    await select("Frequency", "Monthly");
    fireEvent.change(screen.getByLabelText("Day of month"), { target: { value: "32" } });
    expect(screen.getByRole("button", { name: "Confirm schedule" })).toBeDisabled();
    await select("Frequency", "Hourly");
    await select("Repeat every", "4 hours");
    expect(screen.getByRole("status")).toHaveTextContent("Every 4 hours");
    expect(screen.getByRole("button", { name: "Confirm schedule" })).toBeEnabled();
    await select("Frequency", "Once");
    fireEvent.change(screen.getByLabelText("Start date"), { target: { value: "2030-01-01" } });
    expect(screen.queryByLabelText("End date")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Confirm schedule" })).toBeEnabled();
  });
  it("rejects a one-time schedule that expired while the dialog was idle", async () => {
    const clock = vi.spyOn(Date, "now").mockReturnValue(Date.UTC(2030, 0, 1, 8, 0));
    const user = userEvent.setup();
    mount();
    await user.click(screen.getByRole("button", { name: "Schedule a refresh" }));
    await select("Frequency", "Once");
    fireEvent.change(screen.getByLabelText("Timezone"), { target: { value: "UTC" } });
    fireEvent.change(screen.getByLabelText("Start date"), { target: { value: "2030-01-01" } });
    expect(screen.getByRole("button", { name: "Confirm schedule" })).toBeEnabled();
    clock.mockReturnValue(Date.UTC(2030, 0, 1, 10, 0));
    await user.click(screen.getByRole("button", { name: "Confirm schedule" }));
    expect(screen.getByText("The start time has passed. Choose a future date and time.")).toBeInTheDocument();
    expect(getToasts()).toHaveLength(0);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("validates email recipients and subject, then shows a simulated confirmation", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(screen.getByRole("button", { name: "Share via email" }));
    const confirm = screen.getByRole("button", { name: "Confirm email" });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Recipients"), { target: { value: "bad-email" } });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Recipients"), { target: { value: "a@example.com, b@example.com" } });
    fireEvent.change(screen.getByLabelText(/Subject/), { target: { value: "   " } });
    expect(confirm).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/Subject/), { target: { value: "Revenue" } });
    await user.click(confirm);
    expect(screen.getByRole("status")).toHaveTextContent("No email was sent");
    expect(invoke).not.toHaveBeenCalled();
  });
  it("exports for real: the button asks the viewer, and no demo dialog or fake confirmation appears", async () => {
    const user = userEvent.setup();
    mount();
    // The label says what is written, so the reader knows before the dialog opens.
    await user.click(screen.getByRole("button", { name: "Export as an HTML file" }));
    expect(onExport).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.queryByText(/No file was created/)).not.toBeInTheDocument();
    // The renderer calls nothing itself: the viewer owns the native call.
    expect(invoke).not.toHaveBeenCalled();
  });
  it("offers no PDF or PNG: nothing renders them, so they are not a choice", () => {
    mount();
    expect(screen.queryByText(/PDF/)).not.toBeInTheDocument();
    expect(screen.queryByText(/PNG/)).not.toBeInTheDocument();
  });
  it("holds Export while a file is being written, so a second click cannot start a second write", () => {
    mount({ exporting: true });
    expect(screen.getByRole("button", { name: "Export as an HTML file" })).toBeDisabled();
  });
  it("discards cancelled input and restores focus", async () => {
    const user = userEvent.setup();
    mount();
    const share = screen.getByRole("button", { name: "Share via email" });
    await user.click(share);
    await user.type(screen.getByLabelText("Recipients"), "a@example.com");
    await user.keyboard("{Escape}");
    expect(share).toHaveFocus();
    await user.click(share);
    expect(screen.getByLabelText("Recipients")).toHaveValue("");
    expect(invoke).not.toHaveBeenCalled();
  });
});
