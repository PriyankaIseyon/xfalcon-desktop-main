import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act, render, screen, fireEvent, waitFor, cleanup } from "@testing-library/react";
import { DiagnosticsSection } from "./DiagnosticsSection";
import { diagnosticsBundleGenerate, documentExportSave } from "../../lib/native";

vi.mock("../../lib/native", () => ({
  diagnosticsBundleGenerate: vi.fn(),
  documentExportSave: vi.fn(),
}));

afterEach(cleanup);
afterEach(() => {
  vi.useRealTimers();
});

describe("DiagnosticsSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("keeps Support and export controls without redundant introductory copy", () => {
    render(<DiagnosticsSection />);

    const group = screen.getByRole("group", { name: "Support" });
    expect(group).toHaveAttribute("id", "settings-support");
    expect(screen.getByRole("button", { name: "Export diagnostics report" })).toBeInTheDocument();
    // The promise the surface makes, and must keep making: a bundle is written
    // locally and nothing about it leaves the device.
    expect(screen.getByText(/No uploads or network requests/)).toBeInTheDocument();
    // The support line sits at the bottom of the Settings page, not in this block.
    expect(screen.queryByText(/email support@xfalcon.ai/)).not.toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Diagnostics" })).not.toBeInTheDocument();
    expect(screen.queryByText(/workspace and MCP sign-in details/)).not.toBeInTheDocument();
  });

  it("offers an urgent security report that opens the mail app with the build named in it", () => {
    render(<DiagnosticsSection />);

    const link = screen.getByRole("link", { name: /^Report a security issue/ });
    const url = new URL(link.getAttribute("href") ?? "");
    expect(url.protocol).toBe("mailto:");
    expect(url.pathname).toBe("support@xfalcon.ai");
    expect(url.searchParams.get("subject")).toMatch(/^\[URGENT SECURITY\] xFalcon Desktop /);
    expect(url.searchParams.get("body")).toContain("Platform: ");
    expect(link).toHaveAttribute("target", "_blank");
    // The palette jumps here by this id.
    expect(link.closest("[data-settings-row]")).toHaveAttribute("id", "settings-security-report");
  });

  it("invokes bundle generation and saves report via documentExportSave with markdown format", async () => {
    const mockBundle = vi.mocked(diagnosticsBundleGenerate);
    const mockSave = vi.mocked(documentExportSave);

    mockBundle.mockResolvedValue({
      version: 1,
      content: "# Diagnostics Report\n\nOS: Windows",
      suggested_filename: "xfalcon-diagnostics-test",
    });

    mockSave.mockResolvedValue({
      version: 1,
      saved: true,
      file_name: "xfalcon-diagnostics-test.md",
    });

    render(<DiagnosticsSection />);
    const button = screen.getByRole("button", { name: "Export diagnostics report" });
    fireEvent.click(button);

    await waitFor(() => {
      expect(mockBundle).toHaveBeenCalledTimes(1);
      expect(mockSave).toHaveBeenCalledWith(
        "xfalcon-diagnostics-test",
        "# Diagnostics Report\n\nOS: Windows",
        "markdown",
      );
    });

    expect(await screen.findByRole("status")).toHaveTextContent("Diagnostics saved to xfalcon-diagnostics-test.md");
  });

  it("clears a success notice after fifteen seconds", async () => {
    vi.useFakeTimers();
    const mockBundle = vi.mocked(diagnosticsBundleGenerate);
    const mockSave = vi.mocked(documentExportSave);

    mockBundle.mockResolvedValue({
      version: 1,
      content: "# Diagnostics Report",
      suggested_filename: "xfalcon-diagnostics-test",
    });
    mockSave.mockResolvedValue({
      version: 1,
      saved: true,
      file_name: "xfalcon-diagnostics-test.md",
    });

    render(<DiagnosticsSection />);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Export diagnostics report" }));
      await Promise.resolve();
    });

    expect(screen.getByRole("status")).toHaveTextContent("Diagnostics saved to xfalcon-diagnostics-test.md");

    act(() => {
      vi.advanceTimersByTime(15_000);
    });
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("handles cancelled save silently with no error notice", async () => {
    const mockBundle = vi.mocked(diagnosticsBundleGenerate);
    const mockSave = vi.mocked(documentExportSave);

    mockBundle.mockResolvedValue({
      version: 1,
      content: "# Diagnostics Report",
      suggested_filename: "xfalcon-diagnostics-test",
    });

    mockSave.mockResolvedValue({
      version: 1,
      saved: false,
      file_name: null,
    });

    render(<DiagnosticsSection />);
    const button = screen.getByRole("button", { name: "Export diagnostics report" });
    fireEvent.click(button);

    await waitFor(() => {
      expect(mockSave).toHaveBeenCalledWith(expect.anything(), expect.anything(), "markdown");
    });

    expect(screen.queryByRole("status")).toBeNull();
  });

  it("refuses a duplicate export while generation is still in progress", async () => {
    let finish: ((value: { version: 1; content: string; suggested_filename: string }) => void) | undefined;
    vi.mocked(diagnosticsBundleGenerate).mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    vi.mocked(documentExportSave).mockResolvedValueOnce({ version: 1, saved: false, file_name: null });

    render(<DiagnosticsSection />);
    const button = screen.getByRole("button", { name: "Export diagnostics report" });
    fireEvent.click(button);
    fireEvent.click(button);

    expect(diagnosticsBundleGenerate).toHaveBeenCalledTimes(1);
    finish?.({ version: 1, content: "# Diagnostics", suggested_filename: "diagnostics" });
    await waitFor(() => expect(documentExportSave).toHaveBeenCalledTimes(1));
  });
});
