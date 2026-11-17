import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { diagnosticsLogRendererError } from "../lib/native";
import { AppErrorBoundary } from "./AppErrorBoundary";

vi.mock("../lib/native", () => ({
  diagnosticsLogRendererError: vi.fn().mockResolvedValue(undefined),
}));

afterEach(cleanup);

describe("AppErrorBoundary", () => {
  it("shows a redacted minimal recovery surface after a render failure and logs to diagnostics", () => {
    function BrokenSurface(): never {
      throw new Error("token=secret-value at C:\\Users\\person\\private");
    }

    const { container } = render(
      <AppErrorBoundary>
        <BrokenSurface />
      </AppErrorBoundary>,
    );

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("xFalcon needs to recover");
    expect(alert).toHaveTextContent("Reload workspace");
    expect(alert).not.toHaveTextContent("secret-value");
    expect(alert).not.toHaveTextContent("C:\\Users\\person\\private");
    // The same mark the other startup-family surfaces carry -- recovery is
    // reachable straight from a render crash, with no prior screen to have
    // shown it, so this is the only chance this session gets to.
    expect(container.querySelector(".app-mark")).toBeInTheDocument();
    // `.app-recovery h1` used to hand-duplicate the type-title role's own
    // values at higher specificity and supply no font-weight, so this
    // headline rendered unbolded next to every other heading role in the
    // app. `PageTitle` owns the look now.
    expect(screen.getByRole("heading", { name: "xFalcon needs to recover" })).toHaveClass(
      "type-title",
      "font-semibold",
    );
    expect(diagnosticsLogRendererError).toHaveBeenCalledWith(
      expect.objectContaining({
        version: 1,
        message: expect.stringContaining("Error: token=secret-value"),
      }),
    );
  });

  it("resets the failed tree and reports the recovery action", async () => {
    const onReset = vi.fn();
    let shouldFail = true;

    function RecoverableSurface() {
      if (shouldFail) throw new Error("private implementation detail");
      return <p>Workspace restored</p>;
    }

    const user = userEvent.setup();
    render(
      <AppErrorBoundary
        onReset={() => {
          shouldFail = false;
          onReset();
        }}
      >
        <RecoverableSurface />
      </AppErrorBoundary>,
    );

    await user.click(screen.getByRole("button", { name: "Reload workspace" }));

    expect(onReset).toHaveBeenCalledOnce();
    expect(screen.getByText("Workspace restored")).toBeInTheDocument();
  });

  it("reloads the application when no in-memory recovery action is provided", async () => {
    const reload = vi.fn();

    function BrokenSurface(): never {
      throw new Error("private implementation detail");
    }

    const user = userEvent.setup();
    render(
      <AppErrorBoundary reload={reload}>
        <BrokenSurface />
      </AppErrorBoundary>,
    );

    await user.click(screen.getByRole("button", { name: "Reload workspace" }));

    expect(reload).toHaveBeenCalledOnce();
  });
});
