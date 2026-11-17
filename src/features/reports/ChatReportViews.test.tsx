import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ChatToolCallView } from "../../lib/generated/contracts";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  convertFileSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

import { ChatReportViews } from "./ChatReportViews";
import { clearPendingReport, getPendingReportId, resetReportsStore } from "./reportsStore";

const KIT = "6f1c2d3e-0000-4000-8000-000000000001";

const call = (patch: Partial<ChatToolCallView> = {}): ChatToolCallView => ({
  call_id: "call-1",
  name: "mcp_tenant_ida_view_dashboard",
  status: "answered",
  arguments: JSON.stringify({ kit_id: KIT }),
  arguments_truncated: false,
  result: "Showing the saved dashboard.",
  result_truncated: false,
  step: null,
  ...patch,
});

const document = {
  version: 1,
  id: KIT,
  title: "Salesforce Opportunities by Stage",
  description: null,
  page: "index.html",
  pages: ["index.html"],
  document_id: "doc-7",
  datasets: [],
};

beforeEach(() => {
  invoke.mockReset();
  resetReportsStore();
});
afterEach(() => {
  cleanup();
  clearPendingReport();
});

it("draws the saved report a view call showed, from xFalcon, in an isolated frame", async () => {
  invoke.mockResolvedValue(document);
  render(<ChatReportViews calls={[call()]} />);

  const frame = await screen.findByTitle("Salesforce Opportunities by Stage");
  expect(invoke).toHaveBeenCalledWith("report_open", { reportId: KIT, page: null });
  expect(frame.tagName).toBe("IFRAME");
  expect(frame).toHaveAttribute("src", "http://xf-preview.localhost/doc-7");
  expect(frame).toHaveAttribute("sandbox", "allow-scripts");
});

it("opens the report in Reports from the card", async () => {
  invoke.mockResolvedValue(document);
  const user = userEvent.setup();
  render(<ChatReportViews calls={[call()]} />);

  await user.click(await screen.findByRole("button", { name: "Open Salesforce Opportunities by Stage in Reports" }));
  expect(getPendingReportId()).toBe(KIT);
});

it("asks for the named page, and once per report page in a turn", async () => {
  invoke.mockResolvedValue({ ...document, page: "detail.html", pages: ["index.html", "detail.html"] });
  const args = JSON.stringify({ kit_id: KIT, page: "detail.html" });
  render(
    <ChatReportViews calls={[call({ call_id: "a", arguments: args }), call({ call_id: "b", arguments: args })]} />,
  );

  await screen.findByTitle("Salesforce Opportunities by Stage");
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(invoke).toHaveBeenCalledWith("report_open", { reportId: KIT, page: "detail.html" });
  expect(screen.getByText("Saved report · detail.html")).toBeInTheDocument();
});

it("draws nothing for another server's viewer, an unanswered call, or arguments it cannot read", () => {
  render(
    <ChatReportViews
      calls={[
        call({ call_id: "other", name: "mcp_acme_ida_view_dashboard" }),
        call({ call_id: "save", name: "mcp_tenant_ida_save_dashboard" }),
        call({ call_id: "open", status: "unanswered" }),
        call({ call_id: "cut", arguments_truncated: true }),
        call({ call_id: "bad", arguments: "{not json" }),
        call({ call_id: "none", arguments: JSON.stringify({ page: "index.html" }) }),
      ]}
    />,
  );

  expect(invoke).not.toHaveBeenCalled();
  expect(screen.queryByRole("region")).toBeNull();
});

it("points to Reports when the report cannot be shown, and refuses a page answered for another report", async () => {
  invoke.mockRejectedValueOnce({ version: 1, code: "not_found" });
  const first = render(<ChatReportViews calls={[call({ call_id: "gone" })]} />);
  expect(await screen.findByText("This report could not be shown here. Open it from Reports.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Open Saved report in Reports" })).toBeInTheDocument();
  first.unmount();

  invoke.mockResolvedValueOnce({ ...document, id: "someone-else" });
  render(<ChatReportViews calls={[call({ call_id: "swapped" })]} />);
  expect(await screen.findByText("This report could not be shown here. Open it from Reports.")).toBeInTheDocument();
  expect(screen.queryByTitle("Salesforce Opportunities by Stage")).toBeNull();
});

it("shows a turn scrolled away and back without opening the report again, until sign-out", async () => {
  invoke.mockResolvedValue(document);
  const first = render(<ChatReportViews calls={[call({ call_id: "kept" })]} />);
  await screen.findByTitle("Salesforce Opportunities by Stage");
  first.unmount();

  const again = render(<ChatReportViews calls={[call({ call_id: "kept" })]} />);
  await screen.findByTitle("Salesforce Opportunities by Stage");
  expect(invoke).toHaveBeenCalledTimes(1);
  again.unmount();

  resetReportsStore();
  render(<ChatReportViews calls={[call({ call_id: "kept" })]} />);
  await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
});
