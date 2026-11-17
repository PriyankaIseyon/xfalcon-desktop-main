import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ToolInvocationEvent } from "../../lib/generated/contracts";
import { ToolApprovalPrompt } from "./ToolApprovalPrompt";

const resolveMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  chatToolApprovalResolve: resolveMock,
}));

afterEach(() => {
  cleanup();
  resolveMock.mockReset();
});

const approvalRequired: ToolInvocationEvent = {
  type: "approval_required",
  version: 1,
  invocation_id: "inv-1",
  tool_id: "mcp.warehouse.write_table",
};

describe("ToolApprovalPrompt", () => {
  it("shows nothing when no invocation is waiting", () => {
    render(<ToolApprovalPrompt events={[]} runId="run-1" />);

    expect(screen.queryByRole("group", { name: "Tool approvals" })).not.toBeInTheDocument();
  });

  it("names the tool in the same parsed vocabulary as its activity row", () => {
    render(<ToolApprovalPrompt events={[approvalRequired]} runId="run-1" />);

    expect(screen.getByText("write_table")).toBeInTheDocument();
    expect(screen.getByText("· warehouse")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: /Approval required for write_table/ })).toBeInTheDocument();
  });

  it("resolves an approval against the paused invocation from the keyboard", async () => {
    resolveMock.mockResolvedValueOnce(undefined);
    const user = userEvent.setup();
    render(<ToolApprovalPrompt events={[approvalRequired]} runId="run-1" />);

    await user.tab();
    await user.tab();
    expect(screen.getByRole("button", { name: /allow once/i })).toHaveFocus();
    await user.keyboard("{Enter}");

    await waitFor(() => {
      expect(resolveMock).toHaveBeenCalledWith("run-1", "inv-1", true);
    });
    expect(screen.getByRole("button", { name: "Allowed" })).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent("Decision sent");
  });

  it("resolves a decline as a decline, never as an approval", async () => {
    resolveMock.mockResolvedValueOnce(undefined);
    const user = userEvent.setup();
    render(<ToolApprovalPrompt events={[approvalRequired]} runId="run-1" />);

    await user.click(screen.getByRole("button", { name: /deny/i }));

    await waitFor(() => {
      expect(resolveMock).toHaveBeenCalledWith("run-1", "inv-1", false);
    });
  });

  it("stops offering a decision once the invocation moved on", () => {
    // The backend no longer holds this pause, so offering the choice would
    // produce a call that can only fail.
    render(
      <ToolApprovalPrompt
        events={[approvalRequired, { type: "approved", version: 1, invocation_id: "inv-1" }]}
        runId="run-1"
      />,
    );

    expect(screen.queryByRole("group", { name: "Tool approvals" })).not.toBeInTheDocument();
  });

  it("offers nothing when no run is active", () => {
    // Without a run id there is nothing to resolve *against* -- the pause
    // belongs to a specific run's own ToolLoop.
    render(<ToolApprovalPrompt events={[approvalRequired]} runId={null} />);

    expect(screen.queryByRole("group", { name: "Tool approvals" })).not.toBeInTheDocument();
  });

  it("reports a busy decision, restores its action after failure, and permits retry", async () => {
    let rejectDecision!: (cause: unknown) => void;
    resolveMock.mockImplementationOnce(
      () =>
        new Promise<void>((_resolve, reject) => {
          rejectDecision = reject;
        }),
    );
    const user = userEvent.setup();
    render(<ToolApprovalPrompt events={[approvalRequired]} runId="run-1" />);

    await user.click(screen.getByRole("button", { name: /allow once/i }));
    expect(screen.getByRole("button", { name: "Allowing…" })).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("button", { name: "Deny" })).toBeDisabled();

    rejectDecision(new Error("that request is no longer waiting for a decision"));

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent(
        "The decision could not be saved. that request is no longer waiting for a decision",
      );
    });
    expect(screen.getByRole("button", { name: "Allow once" })).toHaveFocus();

    resolveMock.mockResolvedValueOnce(undefined);
    await user.keyboard("{Enter}");
    await waitFor(() => expect(resolveMock).toHaveBeenCalledTimes(2));
  });

  it("does not carry a failed decision into a later run or invocation", async () => {
    resolveMock.mockRejectedValueOnce(new Error("first decision failed"));
    const user = userEvent.setup();
    const { rerender } = render(<ToolApprovalPrompt events={[approvalRequired]} runId="run-1" />);

    await user.click(screen.getByRole("button", { name: /allow once/i }));
    await screen.findByRole("alert");

    rerender(
      <ToolApprovalPrompt
        events={[{ ...approvalRequired, invocation_id: "inv-2", tool_id: "mcp.warehouse.write_rows" }]}
        runId="run-2"
      />,
    );

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Allow once" })).toBeEnabled();
  });
});
