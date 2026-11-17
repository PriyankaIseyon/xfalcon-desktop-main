import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { RunNotice } from "./RunNotice";

afterEach(cleanup);

describe("RunNotice", () => {
  it("renders a resumable run as a quiet status", () => {
    render(<RunNotice message="Resume when ready." />);
    expect(screen.getByRole("status")).toHaveAttribute("data-tone", "info");
    expect(screen.getByText("Resume when ready.")).toBeInTheDocument();
  });

  it("renders a dead run as an alert", () => {
    // A failure has to reach assistive tech without waiting for focus.
    render(<RunNotice message="A tool took too long to answer." tone="error" />);
    expect(screen.getByRole("alert")).toHaveAttribute("data-tone", "danger");
  });

  it("invokes retry and dismiss callbacks", async () => {
    const user = userEvent.setup();
    const onRetry = vi.fn();
    const onDismiss = vi.fn();

    render(<RunNotice message="Interrupted run." onDismiss={onDismiss} onRetry={onRetry} />);

    await user.click(screen.getByRole("button", { name: "Retry run" }));
    await user.click(screen.getByRole("button", { name: "Dismiss" }));

    expect(onRetry).toHaveBeenCalledTimes(1);
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("omits action buttons when handlers are not provided", () => {
    render(<RunNotice message="Interrupted run." />);
    expect(screen.queryByRole("button", { name: "Retry run" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Dismiss" })).not.toBeInTheDocument();
  });

  it("keeps one recovery surface while retry is in flight", () => {
    render(<RunNotice message="Interrupted run." onDismiss={vi.fn()} onRetry={vi.fn()} retrying />);

    expect(screen.getByRole("button", { name: "Retrying…" })).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("button", { name: "Dismiss" })).toBeDisabled();
    expect(screen.getAllByRole("status")).toHaveLength(1);
  });
});
