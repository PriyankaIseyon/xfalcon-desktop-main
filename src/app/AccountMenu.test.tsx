import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { IdentityProjection } from "../lib/generated/contracts";
import { AccountMenu } from "./AccountMenu";

const identity: IdentityProjection = {
  version: 1,
  user_id: "user-test",
  tenant_id: "tenant-test",
  display_name: "Test User",
  email: null,
};

afterEach(cleanup);

function renderMenu(variant: "card" | "compact" = "card") {
  const onSignOut = vi.fn();
  render(
    <AccountMenu
      applicationVersion="1.2.3"
      identity={identity}
      platform="windows"
      variant={variant}
      onSignOut={onSignOut}
    />,
  );
  return onSignOut;
}

describe("AccountMenu", () => {
  it("keeps a useful identity visible in both sidebar densities", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <AccountMenu applicationVersion="1.2.3" identity={identity} platform="windows" onSignOut={() => {}} />,
    );

    expect(screen.getByText("Test User")).toBeInTheDocument();
    expect(screen.queryByText("tenant-test")).not.toBeInTheDocument();

    rerender(
      <AccountMenu
        applicationVersion="1.2.3"
        identity={identity}
        platform="windows"
        variant="compact"
        onSignOut={() => {}}
      />,
    );
    // The compact rail has no room for persistent text. Its identity remains
    // visible through the same portalled help contract as every other rail
    // icon, rather than a local overlay that can clip or drift.
    await user.hover(screen.getByRole("button", { name: "Account and app menu" }));
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Account · Test User");
  });

  it("offers only sign-out in the account menu; local data clear lives in Settings", async () => {
    const user = userEvent.setup();
    render(<AccountMenu applicationVersion="1.2.3" identity={identity} platform="windows" onSignOut={() => {}} />);

    await user.click(screen.getByRole("button", { name: "Account and app menu" }));
    expect(screen.getByRole("menuitem", { name: "Sign out" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Clear local data" })).not.toBeInTheDocument();
    expect(screen.queryByText("Local privacy")).not.toBeInTheDocument();
  });

  it("opens from the keyboard without internal identifiers and signs out", async () => {
    const user = userEvent.setup();
    const onSignOut = renderMenu();
    const trigger = screen.getByRole("button", { name: "Account and app menu" });

    trigger.focus();
    await user.keyboard("{ArrowDown}");

    expect(screen.queryByText("user-test")).not.toBeInTheDocument();
    expect(screen.queryByText("tenant-test")).not.toBeInTheDocument();
    expect(screen.getByRole("menuitem", { name: "Sign out" })).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(onSignOut).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });
});
