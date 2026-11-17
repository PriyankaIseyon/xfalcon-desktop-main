import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ReportView } from "../../lib/generated/contracts";

const capture = vi.hoisted(() => vi.fn());
vi.mock("./reportPreviewCapture", async (original) => ({
  ...(await original<typeof import("./reportPreviewCapture")>()),
  captureReportPreview: capture,
}));

import { ReportActions } from "./ReportActions";
import { ReportPreviewBackfill } from "./ReportPreviewBackfill";

const report = (id: string, patch: Partial<ReportView> = {}) =>
  ({
    id,
    title: id,
    kind: "dashboard",
    is_owner: true,
    thumbnail_updated_at_ms: null,
    pinned: false,
    hidden: false,
    can_manage: true,
    can_delete: true,
    scope: "user",
    ...patch,
  }) as ReportView;

beforeEach(() => {
  capture.mockReset();
});
afterEach(cleanup);

it("offers only the owner's dashboards that have no image, and makes each in turn", async () => {
  const user = userEvent.setup();
  const finishers: Array<() => void> = [];
  capture.mockImplementation(() => new Promise<void>((done) => finishers.push(done)));
  render(
    <ReportPreviewBackfill
      reports={[
        report("a"),
        report("b"),
        report("has-image", { thumbnail_updated_at_ms: 4n }),
        report("not-mine", { is_owner: false }),
        report("workbook", { kind: "workbook" }),
      ]}
    />,
  );

  await user.click(screen.getByRole("button", { name: "Create 2 missing previews" }));
  expect(screen.getByRole("status")).toHaveTextContent("Creating previews · 0 of 2");
  expect(capture).toHaveBeenCalledTimes(1);
  expect(capture.mock.calls[0][0]).toBe("a");

  await act(async () => finishers[0]());
  expect(screen.getByRole("status")).toHaveTextContent("Creating previews · 1 of 2");
  expect(capture.mock.calls[1][0]).toBe("b");
});

it("stops the run when asked, and is absent with nothing to make", async () => {
  const user = userEvent.setup();
  capture.mockImplementation(
    (_id: string, signal: AbortSignal) =>
      new Promise((_, fail) => signal.addEventListener("abort", () => fail(new Error("stopped")))),
  );
  const view = render(<ReportPreviewBackfill reports={[report("a"), report("b")]} />);
  await user.click(screen.getByRole("button", { name: "Create 2 missing previews" }));
  await user.click(screen.getByRole("button", { name: "Stop" }));

  expect(capture).toHaveBeenCalledTimes(1);
  expect(await screen.findByRole("button", { name: "Create 2 missing previews" })).toBeInTheDocument();
  view.rerender(<ReportPreviewBackfill reports={[report("a", { thumbnail_updated_at_ms: 1n })]} />);
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

it("gives the preview action to the owner of a dashboard and to no one else", () => {
  const handlers = {
    onTogglePin: vi.fn(),
    onToggleHidden: vi.fn(),
    onShare: vi.fn(),
    onRename: vi.fn(),
    onDelete: vi.fn(),
    onRegeneratePreview: vi.fn(),
  };
  const { rerender } = render(<ReportActions report={report("a")} {...handlers} />);
  screen.getByRole("button", { name: /^Create preview for/ }).click();
  expect(handlers.onRegeneratePreview).toHaveBeenCalledTimes(1);

  rerender(<ReportActions report={report("a", { thumbnail_updated_at_ms: 2n })} {...handlers} />);
  expect(screen.getByRole("button", { name: /^Update preview of/ })).toBeInTheDocument();
  rerender(<ReportActions report={report("a", { is_owner: false })} {...handlers} />);
  expect(screen.queryByRole("button", { name: /preview/ })).not.toBeInTheDocument();
  rerender(<ReportActions report={report("a", { kind: "presentation" })} {...handlers} />);
  expect(screen.queryByRole("button", { name: /preview/ })).not.toBeInTheDocument();
});
