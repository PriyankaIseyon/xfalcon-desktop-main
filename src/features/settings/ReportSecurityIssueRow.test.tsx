import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { resetPlatformForTests, setPlatform } from "../../lib/platform";
import { ReportSecurityIssueRow } from "./ReportSecurityIssueRow";

// Without this each case finds the previous case's link.
afterEach(cleanup);

describe("ReportSecurityIssueRow", () => {
  beforeEach(() => {
    setPlatform("macos");
  });

  afterEach(() => {
    resetPlatformForTests();
  });

  it("renders a mailto link that the opener hands to the mail app", () => {
    render(<ReportSecurityIssueRow applicationVersion="0.5.37" />);

    const link = screen.getByRole("link", { name: /^Report a security issue/ });
    expect(link.getAttribute("href")).toMatch(/^mailto:support@xfalcon\.ai\?/);
    expect(link).toHaveAttribute("target", "_blank");
    expect(link.getAttribute("rel")).toContain("noopener");
  });

  it("prefills the report with the app version and the platform", () => {
    render(<ReportSecurityIssueRow applicationVersion="0.5.37" />);

    const params = new URL(screen.getByRole("link", { name: /^Report a security issue/ }).getAttribute("href")!)
      .searchParams;
    expect(params.get("subject")).toContain("0.5.37");
    expect(params.get("body")).toContain("Platform: macOS");
  });

  it("shows the support address as copyable text", () => {
    render(<ReportSecurityIssueRow applicationVersion="0.5.37" />);

    expect(screen.getByText(/support@xfalcon\.ai/)).toBeInTheDocument();
  });
});
