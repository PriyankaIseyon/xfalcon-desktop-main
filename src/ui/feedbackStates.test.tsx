import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Alert } from "./Alert";
import { Button } from "./Button";
import { EmptyState } from "./EmptyState";
import { Progress } from "./Progress";
import { Skeleton } from "./Skeleton";
import { Spinner } from "./Spinner";
import { StatusDot } from "./StatusDot";
import { Toaster } from "./Toaster";
import { resetToastsForTests, toast } from "./toastStore";

afterEach(() => {
  cleanup();
  resetToastsForTests();
  vi.useRealTimers();
});

beforeEach(() => resetToastsForTests());

describe("feedback primitives", () => {
  it.each([
    ["info", "status"],
    ["success", "status"],
    ["warn", "status"],
    ["danger", "alert"],
  ] as const)("gives the %s alert tone text, an icon, and the %s role", (tone, role) => {
    render(
      <Alert action={<Button>Try again</Button>} title="Import failed" tone={tone}>
        Keep this file and check the format.
      </Alert>,
    );

    const alert = screen.getByRole(role);
    expect(alert).toHaveAttribute("data-tone", tone);
    expect(alert).toHaveTextContent("Import failed");
    expect(alert).toHaveTextContent("Keep this file and check the format.");
    expect(within(alert).getByRole("button", { name: "Try again" })).toBeInTheDocument();
    expect(alert.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
  });

  it("keeps one three-card toast stack and lets older notifications wait", () => {
    render(<Toaster />);
    act(() => {
      toast.message("First", null);
      toast.success("Second", null);
      toast.warning("Third", null);
      toast.error("Fourth", null);
    });

    expect(screen.queryByText("First")).not.toBeInTheDocument();
    expect(screen.getByText("Second")).toBeInTheDocument();
    expect(screen.getByText("Third")).toBeInTheDocument();
    expect(screen.getByText("Fourth").closest("[role]")).toHaveAttribute("role", "alert");
    expect(screen.getAllByLabelText("Dismiss notification")).toHaveLength(3);

    fireEvent.click(screen.getAllByLabelText("Dismiss notification").at(-1)!);
    expect(screen.getByText("First")).toBeInTheDocument();
  });

  it("gives a toast that leaves, returns, and leaves again one full fade", () => {
    vi.useFakeTimers();
    render(<Toaster />);
    act(() => {
      toast.message("First", null);
      toast.message("Second", null);
      toast.message("Third", null);
    });
    // A fourth pushes "First" out of the stack, and it starts to fade.
    act(() => {
      toast.message("Fourth", null);
    });
    act(() => vi.advanceTimersByTime(300));
    // Dismissing the newest brings "First" back mid-fade; a fifth pushes it
    // out again.
    fireEvent.click(screen.getAllByLabelText("Dismiss notification").at(-1)!);
    act(() => {
      toast.message("Fifth", null);
    });
    expect(screen.getAllByText("First")).toHaveLength(1);

    // The first exit's timer would have fired here and cut the second fade.
    act(() => vi.advanceTimersByTime(300));
    expect(screen.getAllByText("First")).toHaveLength(1);
    act(() => vi.advanceTimersByTime(200));
    expect(screen.queryByText("First")).not.toBeInTheDocument();
  });

  it("keeps the toast action before dismiss and fades the card out after the action", () => {
    vi.useFakeTimers();
    const onSelect = vi.fn();
    render(<Toaster />);
    act(() => {
      toast.error("Upload stopped.", null, { label: "Try again", onSelect });
    });

    const alert = screen.getByRole("alert");
    const controls = within(alert).getAllByRole("button");
    expect(controls.map((control) => control.getAttribute("aria-label") ?? control.textContent)).toEqual([
      "Try again",
      "Dismiss notification",
    ]);
    fireEvent.click(controls[0]);
    expect(onSelect).toHaveBeenCalledOnce();
    // Dismissed, but not gone yet: the same card fades out rather than
    // vanishing on the frame the action fired.
    expect(alert).toHaveAttribute("data-exiting", "true");
    expect(screen.getByText("Upload stopped.")).toBeInTheDocument();
    // Still fading just short of the 500ms overlay exit, gone once it ends.
    act(() => vi.advanceTimersByTime(450));
    expect(screen.getByText("Upload stopped.")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(50));
    expect(screen.queryByText("Upload stopped.")).not.toBeInTheDocument();
  });

  it("fades a directly-dismissed toast out instead of removing it instantly", () => {
    vi.useFakeTimers();
    render(<Toaster />);
    act(() => {
      toast.message("Draft saved", null);
    });

    const card = screen.getByText("Draft saved").closest(".app-toaster__toast")!;
    expect(card).not.toHaveAttribute("data-exiting");

    fireEvent.click(screen.getByLabelText("Dismiss notification"));
    expect(card).toHaveAttribute("data-exiting", "true");
    expect(card).toHaveAttribute("aria-hidden", "true");
    expect(screen.getByText("Draft saved")).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1_000));
    expect(screen.queryByText("Draft saved")).not.toBeInTheDocument();
  });

  it("drops a dismissed toast at once under reduced motion, where no fade plays", () => {
    vi.useFakeTimers();
    const matchMedia = vi.fn((query: string) => ({ matches: query.includes("reduce") }));
    vi.stubGlobal("matchMedia", matchMedia);
    try {
      render(<Toaster />);
      act(() => {
        toast.message("Saved");
      });
      fireEvent.click(screen.getByLabelText("Dismiss notification"));
      act(() => vi.advanceTimersByTime(0));
      expect(screen.queryByText("Saved")).not.toBeInTheDocument();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("pauses auto-dismiss while the pointer is over the stack", () => {
    vi.useFakeTimers();
    render(<Toaster />);
    act(() => {
      toast.message("Saved");
    });
    const stack = screen.getByText("Saved").parentElement!.parentElement!;

    fireEvent.pointerEnter(stack);
    act(() => vi.advanceTimersByTime(4_000));
    expect(screen.getByText("Saved")).toBeInTheDocument();
    fireEvent.pointerLeave(stack);
    // Two steps, not one bigger number: the first clears the resumed
    // auto-dismiss timer and starts the exit fade; React only commits that
    // state change (and the effect that schedules the exit-fade's own
    // removal timer, `EXIT_MS` in `Toaster.tsx`) once this `act()` returns,
    // so the removal timer does not exist yet for a single combined advance
    // to find. The second step clears that now-scheduled timer.
    act(() => vi.advanceTimersByTime(3_000));
    act(() => vi.advanceTimersByTime(500));
    expect(screen.queryByText("Saved")).not.toBeInTheDocument();
  });

  it("pauses auto-dismiss while keyboard focus is in the stack", () => {
    vi.useFakeTimers();
    render(<Toaster />);
    act(() => {
      toast.success("Report saved");
    });
    const dismiss = screen.getByLabelText("Dismiss notification");

    fireEvent.focus(dismiss);
    act(() => vi.advanceTimersByTime(4_000));
    expect(screen.getByText("Report saved")).toBeInTheDocument();
    fireEvent.blur(dismiss, { relatedTarget: null });
    // Two steps, not one bigger number: the first clears the resumed
    // auto-dismiss timer and starts the exit fade; React only commits that
    // state change (and the effect that schedules the exit-fade's own
    // removal timer, `EXIT_MS` in `Toaster.tsx`) once this `act()` returns,
    // so the removal timer does not exist yet for a single combined advance
    // to find. The second step clears that now-scheduled timer.
    act(() => vi.advanceTimersByTime(3_000));
    act(() => vi.advanceTimersByTime(500));
    expect(screen.queryByText("Report saved")).not.toBeInTheDocument();
  });

  it("stays paused until both pointer and keyboard focus leave the stack", () => {
    vi.useFakeTimers();
    render(<Toaster />);
    act(() => {
      toast.message("Draft saved");
    });
    const message = screen.getByText("Draft saved");
    const stack = message.parentElement!.parentElement!;
    const dismiss = screen.getByLabelText("Dismiss notification");

    fireEvent.pointerEnter(stack);
    fireEvent.focus(dismiss);
    fireEvent.pointerLeave(stack);
    act(() => vi.advanceTimersByTime(4_000));
    expect(screen.getByText("Draft saved")).toBeInTheDocument();
    fireEvent.blur(dismiss, { relatedTarget: null });
    // Two steps, not one bigger number: the first clears the resumed
    // auto-dismiss timer and starts the exit fade; React only commits that
    // state change (and the effect that schedules the exit-fade's own
    // removal timer, `EXIT_MS` in `Toaster.tsx`) once this `act()` returns,
    // so the removal timer does not exist yet for a single combined advance
    // to find. The second step clears that now-scheduled timer.
    act(() => vi.advanceTimersByTime(3_000));
    act(() => vi.advanceTimersByTime(500));
    expect(screen.queryByText("Draft saved")).not.toBeInTheDocument();
  });

  it("keeps an error notification for 15 seconds, and one asked to stay until it is dismissed", () => {
    vi.useFakeTimers();
    render(<Toaster />);
    act(() => {
      toast.error("Connection failed. Try again.");
      toast.error("Upload stopped.", null);
    });
    act(() => vi.advanceTimersByTime(14_900));
    expect(screen.getByText("Connection failed. Try again.")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(100));
    act(() => vi.advanceTimersByTime(500));
    expect(screen.queryByText("Connection failed. Try again.")).not.toBeInTheDocument();
    act(() => vi.advanceTimersByTime(60_000));
    expect(screen.getByText("Upload stopped.")).toBeInTheDocument();
  });

  it("states determinate, complete, and indeterminate progress without color", () => {
    const { rerender } = render(<Progress label="Uploading quarterly data" value={38} />);
    const progress = screen.getByRole("progressbar", { name: "Uploading quarterly data" });
    expect(progress).toHaveAttribute("data-state", "progress");
    expect(progress).toHaveAttribute("aria-valuenow", "38");
    expect(progress).toHaveAttribute("aria-valuetext", "38%");

    rerender(<Progress label="Upload complete" tone="success" value={100} />);
    expect(screen.getByRole("progressbar", { name: "Upload complete" })).toHaveAttribute("data-state", "complete");

    rerender(<Progress indeterminate label="Preparing upload" value={0} />);
    const pending = screen.getByRole("progressbar", { name: "Preparing upload" });
    expect(pending).toHaveAttribute("data-state", "indeterminate");
    expect(pending).not.toHaveAttribute("aria-valuenow");
    expect(pending).toHaveAttribute("aria-valuetext", "Preparing upload");
  });

  it("keeps skeletons hidden and spinners either named or explicitly decorative", () => {
    const { container } = render(
      <div>
        <Skeleton aria-hidden={false} className="h-8" />
        <Spinner label="Reading schema" />
        <Spinner decorative />
      </div>,
    );

    expect(container.querySelector(".skeleton-shimmer")).toHaveAttribute("aria-hidden", "true");
    expect(screen.getByRole("status")).toHaveTextContent("Reading schema");
    expect(container.querySelectorAll("svg[aria-hidden='true']")).toHaveLength(2);
  });

  it("keeps a composed status dot decorative and names a standalone dot", () => {
    const { container } = render(
      <div>
        <StatusDot decorative tone="online" />
        <StatusDot label="Sync failed" tone="error" />
      </div>,
    );

    expect(container.querySelector("[aria-hidden='true']")).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Sync failed" })).toHaveTextContent("Sync failed");
  });

  it("names an empty place, explains it, and keeps its next action", () => {
    render(
      <EmptyState
        actions={<Button>Upload dataset</Button>}
        description="This workspace has no datasets yet. Add a CSV or Excel file to start."
        title="No datasets yet"
      />,
    );

    expect(screen.getByRole("heading", { name: "No datasets yet" })).toBeInTheDocument();
    expect(screen.getByText(/This workspace has no datasets yet/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Upload dataset" })).toBeInTheDocument();
  });
});
