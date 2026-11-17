import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { McpServerUnavailableReason, McpServerView } from "../../lib/generated/contracts";
import { McpServersSection } from "./McpServersSection";
import { toast } from "../../ui/toastStore";

const mcpServerListMock = vi.hoisted(() => vi.fn());
const mcpServerDeleteMock = vi.hoisted(() => vi.fn());
const mcpServerRestoreBundledMock = vi.hoisted(() => vi.fn());
const mcpServerCreateMock = vi.hoisted(() => vi.fn());
const mcpServerUpdateMock = vi.hoisted(() => vi.fn());
const mcpServerTestMock = vi.hoisted(() => vi.fn());
const mcpServerSignInMock = vi.hoisted(() => vi.fn());
const mcpServerSignOutMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  mcpServerList: mcpServerListMock,
  mcpServerDelete: mcpServerDeleteMock,
  mcpServerRestoreBundled: mcpServerRestoreBundledMock,
  mcpServerCreate: mcpServerCreateMock,
  mcpServerUpdate: mcpServerUpdateMock,
  mcpServerTest: mcpServerTestMock,
  mcpServerSignIn: mcpServerSignInMock,
  mcpServerSignOut: mcpServerSignOutMock,
}));

const bundled: McpServerView = {
  version: 1,
  id: "tenant",
  name: "tenant",
  display_name: "Tenant Data Analyst",
  description: "Your tenant's bundled analytical workspace. Connected automatically after sign-in.",
  transport: "http",
  url: "https://tenant.example/mcp",
  auth_type: "bearer",
  permission_mode: "ask",
  enabled: true,
  builtin: true,
  slug: "tenant",
  unavailable: null,
  sign_in: null,
  oauth_client_id: null,
  oauth_client_confidential: false,
};

function registered(overrides: Partial<McpServerView> = {}): McpServerView {
  return {
    version: 1,
    id: "srv-1",
    name: "acme-crm",
    display_name: "Acme CRM",
    description: "Customer records",
    transport: "http",
    url: "https://mcp.acme.example/mcp",
    auth_type: "bearer",
    permission_mode: "ask",
    enabled: true,
    builtin: false,
    slug: "acme-crm",
    unavailable: null,
    sign_in: null,
    oauth_client_id: null,
    oauth_client_confidential: false,
    ...overrides,
  };
}

function listResolves(servers: McpServerView[]) {
  mcpServerListMock.mockResolvedValue({ version: 1, servers });
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  mcpServerListMock.mockReset();
  mcpServerDeleteMock.mockReset();
  mcpServerRestoreBundledMock.mockReset();
  mcpServerCreateMock.mockReset();
  mcpServerUpdateMock.mockReset();
  mcpServerTestMock.mockReset();
  mcpServerSignInMock.mockReset();
  mcpServerSignOutMock.mockReset();
});

describe("McpServersSection", () => {
  it("removes and restores the bundled connection without removing other servers", async () => {
    listResolves([bundled, registered()]);
    mcpServerDeleteMock.mockImplementationOnce(async () => {
      listResolves([registered()]);
    });
    mcpServerRestoreBundledMock.mockImplementationOnce(async () => {
      listResolves([bundled, registered()]);
      return bundled;
    });
    const user = userEvent.setup();
    const onRegistryChanged = vi.fn();
    render(<McpServersSection onRegistryChanged={onRegistryChanged} />);
    const title = await screen.findByText("Workspace MCP server");
    const card = title.closest(".settings-collection-item");
    expect(card).not.toBeNull();
    await user.click(within(card as HTMLElement).getByRole("button", { name: "Remove Workspace MCP server" }));
    const dialog = screen.getByRole("alertdialog");
    expect(dialog).toHaveTextContent("The xFalcon account stays signed in");
    expect(mcpServerDeleteMock).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: "Remove server" }));
    await waitFor(() => expect(mcpServerDeleteMock).toHaveBeenCalledExactlyOnceWith("tenant"));
    const restore = await screen.findByRole("button", { name: "Restore workspace MCP server" });
    expect(screen.queryByText("Workspace MCP server")).not.toBeInTheDocument();
    expect(screen.getByText("Acme CRM")).toBeInTheDocument();
    await user.click(restore);
    expect(await screen.findByText("Workspace MCP server")).toBeInTheDocument();
    expect(screen.getByText("Acme CRM")).toBeInTheDocument();
    expect(mcpServerRestoreBundledMock).toHaveBeenCalledOnce();
    expect(onRegistryChanged).toHaveBeenCalledTimes(2);
  });

  it("keeps a failed bundled removal visible and reports a failed restore", async () => {
    listResolves([bundled]);
    mcpServerDeleteMock.mockRejectedValueOnce({ version: 1, code: "storage_unavailable" });
    const user = userEvent.setup();
    const view = render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Remove Workspace MCP server" }));
    await user.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Remove server" }));
    expect(await screen.findByRole("alert")).toBeVisible();
    expect(screen.getByText("Workspace MCP server")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Restore workspace MCP server" })).not.toBeInTheDocument();
    view.unmount();
    listResolves([]);
    mcpServerRestoreBundledMock.mockRejectedValueOnce({ version: 1, code: "storage_unavailable" });
    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Restore workspace MCP server" }));
    expect(await screen.findByRole("alert")).toBeVisible();
    expect(screen.queryByText("Workspace MCP server")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Restore workspace MCP server" })).toBeEnabled();
  });

  it("shows the packaged tenant endpoint with removal separate from account sign-in", async () => {
    listResolves([bundled]);

    render(<McpServersSection />);

    expect(await screen.findByText("Workspace MCP server")).toBeInTheDocument();
    expect(screen.getByText("Signed in")).toBeInTheDocument();
    expect(screen.getByText("https://tenant.example/mcp")).toBeInTheDocument();
    expect(screen.getByText("Streamable HTTP · Browser sign-in")).toBeInTheDocument();
    expect(await screen.findByText("No additional servers")).toBeInTheDocument();
    expect(screen.queryByText("Not signed in on this device")).not.toBeInTheDocument();
    expect(screen.queryByText("Not being used on this device")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Edit Workspace MCP server" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove Workspace MCP server" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Sign out" })).not.toBeInTheDocument();
  });

  it("explains a local-process server as something with no endpoint to dial", async () => {
    listResolves([registered({ transport: "stdio", unavailable: "unsupported_transport" })]);

    render(<McpServersSection />);

    expect(await screen.findByText(/requires a remote HTTPS endpoint/)).toBeInTheDocument();
    expect(screen.getByText("Local process")).toBeInTheDocument();
    expect(screen.getByText(/Local process connections are not supported/)).toBeInTheDocument();
  });

  // Every server registered from xFalcon Web is labelled `sse` regardless of what
  // its endpoint speaks, so refusing the label refused all of them.
  it("uses a server labelled HTTP+SSE without claiming a connection was tested", async () => {
    listResolves([registered({ transport: "sse", unavailable: null })]);
    const user = userEvent.setup();

    render(<McpServersSection />);

    expect(await screen.findByText("Acme CRM")).toBeInTheDocument();
    expect(screen.queryByText("Not being used on this device")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show details" }));
    expect(screen.getByText(/Desktop uses Streamable HTTP/)).toBeInTheDocument();
    expect(screen.queryByText(/Connected with Streamable HTTP/)).not.toBeInTheDocument();
  });

  it("explains a method outside this build's vocabulary without blaming the server", async () => {
    listResolves([registered({ auth_type: "other", unavailable: "unsupported_auth_method" })]);

    render(<McpServersSection />);

    expect(await screen.findByText("This authentication method is not supported.")).toBeInTheDocument();
    expect(screen.getByText(/Supported methods are OAuth/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sign in with this server" })).not.toBeInTheDocument();
  });

  it("offers a browser sign-in for an OAuth server this device has not signed in to", async () => {
    const signedIn = registered({ auth_type: "oauth", unavailable: null, sign_in: "signed_in" });
    listResolves([registered({ auth_type: "oauth", unavailable: "needs_sign_in", sign_in: "not_signed_in" })]);
    mcpServerSignInMock.mockResolvedValueOnce(signedIn);
    const onRegistryChanged = vi.fn();
    const user = userEvent.setup();

    render(<McpServersSection onRegistryChanged={onRegistryChanged} />);

    expect(await screen.findByText("Sign in to this server on this device to use it.")).toBeInTheDocument();
    expect(screen.getByText("Not in use")).toBeInTheDocument();
    // The sign-in is per device even though the definition is shared, and the
    // copy has to say so or a user will expect one sign-in to cover every machine.
    expect(screen.getByText(/Credentials remain on that device/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Sign in with this server" }));

    // No client details are asked for up front: most servers issue a client on
    // request, and asking first would make the common case look harder.
    await waitFor(() => expect(mcpServerSignInMock).toHaveBeenCalledWith("srv-1", null, null));
    expect(await screen.findByText("Signed in on this device")).toBeInTheDocument();
    expect(screen.queryByText("Not being used on this device")).not.toBeInTheDocument();
    expect(onRegistryChanged).toHaveBeenCalledOnce();
  });

  it("asks for the operator-issued client ID only after a sign-in reports one is needed", async () => {
    listResolves([registered({ auth_type: "oauth", unavailable: "needs_sign_in", sign_in: "not_signed_in" })]);
    mcpServerSignInMock
      .mockRejectedValueOnce({ version: 1, code: "client_registration_unavailable" })
      .mockResolvedValueOnce(registered({ auth_type: "oauth", unavailable: null, sign_in: "signed_in" }));
    const user = userEvent.setup();

    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Sign in with this server" }));

    const dialog = await screen.findByRole("dialog", { name: "Sign in to “Acme CRM”" });
    expect(within(dialog).getByText(/client ID supplied by the server administrator/)).toBeInTheDocument();

    await user.type(within(dialog).getByLabelText("Client ID"), "  connector-client  ");
    expect(within(dialog).getByLabelText("Client secret")).toHaveValue("");
    await user.click(within(dialog).getByRole("button", { name: "Sign in" }));

    await waitFor(() => expect(mcpServerSignInMock).toHaveBeenLastCalledWith("srv-1", "connector-client", null));
    expect(await screen.findByText("Signed in on this device")).toBeInTheDocument();
  });

  it("shows a workspace-wide sign-in as shared, and confirms before taking it from everyone", async () => {
    listResolves([registered({ auth_type: "oauth", unavailable: null, sign_in: "signed_in_for_account" })]);
    mcpServerSignOutMock.mockResolvedValueOnce(
      registered({ auth_type: "oauth", unavailable: "needs_sign_in", sign_in: "not_signed_in" }),
    );
    const user = userEvent.setup();

    render(<McpServersSection />);

    expect(await screen.findByText("Signed in for the workspace")).toBeInTheDocument();
    expect(screen.getByText(/Shared across xFalcon clients in the workspace/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Sign out" }));

    // Reach is the reason for the confirmation: this takes the grant away from
    // other people, unlike a device sign-out.
    expect(mcpServerSignOutMock).not.toHaveBeenCalled();
    const dialog = screen.getByRole("alertdialog", { name: "Sign out of “Acme CRM”?" });
    expect(within(dialog).getByText(/Signs out all clients on this account/)).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "Sign out everyone" }));

    await waitFor(() => expect(mcpServerSignOutMock).toHaveBeenCalledWith("srv-1"));
    expect(await screen.findByText("Not in use")).toBeInTheDocument();
  });

  it("keeps a workspace-wide sign-in when the confirmation is declined", async () => {
    listResolves([registered({ auth_type: "oauth", unavailable: null, sign_in: "signed_in_for_account" })]);
    const user = userEvent.setup();

    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Sign out" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(mcpServerSignOutMock).not.toHaveBeenCalled();
    expect(screen.getByText("Signed in for the workspace")).toBeInTheDocument();
  });

  it("retains a failed workspace sign-out for retry", async () => {
    listResolves([registered({ auth_type: "oauth", unavailable: null, sign_in: "signed_in_for_account" })]);
    mcpServerSignOutMock.mockRejectedValueOnce({ version: 1, code: "service_unavailable" });
    const user = userEvent.setup();

    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Sign out" }));
    await user.click(screen.getByRole("button", { name: "Sign out everyone" }));

    const dialog = await screen.findByRole("alertdialog", { name: "Sign out of “Acme CRM”?" });
    expect(dialog).toHaveTextContent("The server registry could not be reached. Try again in a moment.");
    expect(screen.getByRole("button", { name: "Sign out everyone" })).toBeEnabled();
    expect(screen.getByText("Signed in for the workspace")).toBeInTheDocument();
  });

  it("signs out of one server without touching the shared definition", async () => {
    listResolves([registered({ auth_type: "oauth", unavailable: null, sign_in: "signed_in" })]);
    mcpServerSignOutMock.mockResolvedValueOnce(
      registered({ auth_type: "oauth", unavailable: "needs_sign_in", sign_in: "not_signed_in" }),
    );
    const user = userEvent.setup();

    render(<McpServersSection />);
    expect(await screen.findByText(/Applies to this device only/)).toBeInTheDocument();

    // A device-only sign-out affects nobody else, so it needs no confirmation.
    await user.click(screen.getByRole("button", { name: "Sign out" }));

    await waitFor(() => expect(mcpServerSignOutMock).toHaveBeenCalledWith("srv-1"));
    expect(await screen.findByText("Not in use")).toBeInTheDocument();
    expect(mcpServerDeleteMock).not.toHaveBeenCalled();
  });

  it("explains when device sign-out leaves an account grant available and confirms its removal separately", async () => {
    listResolves([registered({ auth_type: "oauth", sign_in: "signed_in" })]);
    mcpServerSignOutMock.mockResolvedValueOnce(registered({ auth_type: "oauth", sign_in: "signed_in_for_account" }));
    const success = vi.spyOn(toast, "success");
    const user = userEvent.setup();
    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Sign out" }));
    await waitFor(() =>
      expect(success).toHaveBeenCalledWith(expect.stringContaining("Workspace sign-in remains available")),
    );
    await user.click(screen.getByRole("button", { name: "Sign out" }));
    expect(screen.getByRole("alertdialog")).toHaveTextContent("Signs out all clients on this account");
    expect(mcpServerSignOutMock).toHaveBeenCalledTimes(1);
  });

  it("starts OAuth sign-in immediately after creating an oauth server", async () => {
    const created = registered({
      id: "srv-oauth",
      name: "oauth-server",
      display_name: "OAuth Server",
      auth_type: "oauth",
      unavailable: "needs_sign_in",
      sign_in: "not_signed_in",
    });
    const signedIn = { ...created, sign_in: "signed_in" as const, unavailable: null };
    listResolves([]);
    mcpServerCreateMock.mockResolvedValueOnce(created);
    mcpServerSignInMock.mockResolvedValueOnce(signedIn);
    const user = userEvent.setup();

    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Add server" }));

    const sheet = screen.getByRole("dialog", { name: "Add MCP server" });
    await user.type(within(sheet).getByLabelText(/^Name/), "oauth-server");
    await user.type(within(sheet).getByLabelText(/^Server address/), "https://mcp.example.test/mcp");
    expect(within(sheet).getByLabelText(/^Client ID/)).toHaveValue("");
    expect(within(sheet).getByLabelText(/^Client secret/)).toHaveValue("");
    await user.type(within(sheet).getByLabelText(/^Client ID/), "issued-client");
    await user.type(within(sheet).getByLabelText(/^Client secret/), "issued-secret");
    await user.click(within(sheet).getByRole("button", { name: "Add server" }));

    await waitFor(() => expect(mcpServerCreateMock).toHaveBeenCalledOnce());
    await waitFor(() => expect(mcpServerSignInMock).toHaveBeenCalledWith("srv-oauth", null, null));
    expect(await screen.findByText("Signed in on this device")).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "Add MCP server" })).not.toBeInTheDocument();
  });

  it("stores the supplied client ID and secret, then signs in from what was stored", async () => {
    const secret = "issued-secret";
    const created = registered({
      auth_type: "oauth",
      sign_in: "not_signed_in",
      unavailable: "needs_sign_in",
      oauth_client_id: "issued-client",
      oauth_client_confidential: true,
    });
    listResolves([]);
    mcpServerCreateMock.mockResolvedValueOnce(created);
    mcpServerSignInMock.mockResolvedValueOnce({ ...created, sign_in: "signed_in", unavailable: null });
    const user = userEvent.setup();
    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Add server" }));
    const sheet = screen.getByRole("dialog", { name: "Add MCP server" });
    await waitFor(() => expect(within(sheet).getByRole("button", { name: "Close" })).toHaveFocus());
    await user.type(within(sheet).getByLabelText(/^Name/), "acme-crm");
    await user.type(within(sheet).getByLabelText(/^Server address/), "https://mcp.acme.example/mcp");
    await user.type(within(sheet).getByLabelText(/^Client ID/), "issued-client");
    await user.type(within(sheet).getByLabelText(/^Client secret/), secret);
    await user.click(within(sheet).getByRole("button", { name: "Add server" }));

    // Creating is what persists the pair; sign-in then reads it from the
    // device store, which is what makes a declined flow retryable without
    // typing it again.
    expect(mcpServerCreateMock).toHaveBeenCalledWith(expect.objectContaining({ auth_type: "oauth" }), {
      clientId: "issued-client",
      clientSecret: secret,
    });
    await waitFor(() => expect(mcpServerSignInMock).toHaveBeenCalledExactlyOnceWith("srv-1", null, null));
    expect(await screen.findByText("Signed in on this device")).toBeInTheDocument();
  });

  it("reopens browser sign-in after the stored OAuth client changes, and not otherwise", async () => {
    const current = registered({
      auth_type: "oauth",
      sign_in: "signed_in",
      oauth_client_id: "issued-client",
      oauth_client_confidential: true,
    });
    const reissued = { ...current, oauth_client_id: "reissued-client" };
    listResolves([current]);
    mcpServerUpdateMock.mockResolvedValueOnce(reissued);
    mcpServerSignInMock.mockResolvedValueOnce(reissued);
    const user = userEvent.setup();
    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Edit Acme CRM" }));
    const sheet = screen.getByRole("dialog", { name: "Edit MCP server" });
    await waitFor(() => expect(within(sheet).getByRole("button", { name: "Close" })).toHaveFocus());
    await user.clear(within(sheet).getByLabelText(/^Client ID/));
    await user.type(within(sheet).getByLabelText(/^Client ID/), "reissued-client");
    await user.type(within(sheet).getByLabelText(/^Client secret/), "reissued-secret");
    await user.click(within(sheet).getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(mcpServerSignInMock).toHaveBeenCalledExactlyOnceWith("srv-1", null, null));
  });

  it("does not reopen sign-in when an edit leaves the connection and its client alone", async () => {
    const current = registered({
      auth_type: "oauth",
      sign_in: "signed_in",
      oauth_client_id: "issued-client",
      oauth_client_confidential: true,
    });
    listResolves([current]);
    mcpServerUpdateMock.mockResolvedValueOnce({ ...current, description: "Updated notes" });
    const user = userEvent.setup();
    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Edit Acme CRM" }));
    const sheet = screen.getByRole("dialog", { name: "Edit MCP server" });
    await waitFor(() => expect(within(sheet).getByRole("button", { name: "Close" })).toHaveFocus());
    await user.clear(within(sheet).getByLabelText("Description"));
    await user.type(within(sheet).getByLabelText("Description"), "Updated notes");
    await user.click(within(sheet).getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(mcpServerUpdateMock).toHaveBeenCalledOnce());
    // A description edit is not a reason to send the user back to a browser.
    expect(mcpServerSignInMock).not.toHaveBeenCalled();
  });

  it("reopens browser sign-in after changing an OAuth server's address", async () => {
    const current = registered({
      auth_type: "oauth",
      sign_in: "signed_in",
      oauth_client_id: "issued-client",
      oauth_client_confidential: true,
    });
    const updated = { ...current, url: "https://replacement.example/mcp" };
    listResolves([current]);
    mcpServerUpdateMock.mockResolvedValueOnce(updated);
    mcpServerSignInMock.mockResolvedValueOnce(updated);
    const user = userEvent.setup();
    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Edit Acme CRM" }));
    const sheet = screen.getByRole("dialog", { name: "Edit MCP server" });
    await waitFor(() => expect(within(sheet).getByRole("button", { name: "Close" })).toHaveFocus());
    await user.clear(within(sheet).getByLabelText(/^Server address/));
    await user.type(within(sheet).getByLabelText(/^Server address/), updated.url);
    await user.click(within(sheet).getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(mcpServerSignInMock).toHaveBeenCalledExactlyOnceWith("srv-1", null, null));
  });

  it("turns a declined sign-in into a sentence on that row and leaves it signed out", async () => {
    listResolves([registered({ auth_type: "oauth", unavailable: "needs_sign_in", sign_in: "not_signed_in" })]);
    mcpServerSignInMock.mockRejectedValueOnce({ version: 1, code: "sign_in_declined" });
    const user = userEvent.setup();

    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Sign in with this server" }));

    expect(
      await screen.findByText("Sign-in was not completed in the browser. Nothing was changed."),
    ).toBeInTheDocument();
    expect(screen.getByText("Not in use")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("shows no sign-in affordance for a server that authenticates with a pasted value", async () => {
    listResolves([registered({ auth_type: "bearer", sign_in: null })]);

    render(<McpServersSection />);

    expect(await screen.findByText("Acme CRM")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sign out" })).not.toBeInTheDocument();
    expect(screen.queryByText("Signed in on this device")).not.toBeInTheDocument();
  });

  it("explains a missing device credential without offering a form that cannot collect one", async () => {
    listResolves([registered({ auth_type: "bearer", unavailable: "missing_device_credential" })]);

    render(<McpServersSection />);

    expect(await screen.findByText("Add this server's credential on this device to use it.")).toBeInTheDocument();
    // The definition is shared, the credential is not -- the copy must not imply
    // the server itself is broken.
    expect(screen.getByText(/Credentials must be added separately on each device/)).toBeInTheDocument();
    expect(screen.getByText(/The server is shared across the account/)).toBeInTheDocument();
    // This app registers browser sign-in only, so it has nowhere to put a static
    // credential. A row that needs one was registered by another client and is
    // managed there; offering a button that opens a sheet without the field
    // would be an action that cannot complete.
    expect(screen.queryByRole("button", { name: "Add credential on this device" })).not.toBeInTheDocument();
  });

  it("explains a rejected address without repeating the raw URL policy", async () => {
    listResolves([registered({ url: "http://10.0.0.5/mcp", unavailable: "invalid_url" })]);

    render(<McpServersSection />);

    expect(await screen.findByText("This server's address cannot be used from the desktop app.")).toBeInTheDocument();
    expect(screen.getByText(/must be reachable at an https address on a public host/)).toBeInTheDocument();
  });

  it("states what each permission mode does to approvals on every row", async () => {
    listResolves([registered({ permission_mode: "allow" })]);
    const user = userEvent.setup();

    render(<McpServersSection />);

    await user.click(await screen.findByRole("button", { name: "Show details" }));
    expect(screen.getByText(/Tools that can change data require approval/)).toBeInTheDocument();
  });

  it("requires a confirmation before removing a server, and says what removal costs", async () => {
    listResolves([registered()]);
    mcpServerDeleteMock.mockResolvedValueOnce(undefined);
    const onRegistryChanged = vi.fn();
    const user = userEvent.setup();

    render(<McpServersSection onRegistryChanged={onRegistryChanged} />);
    await user.click(await screen.findByRole("button", { name: "Remove Acme CRM" }));

    expect(mcpServerDeleteMock).not.toHaveBeenCalled();
    const dialog = screen.getByRole("alertdialog", { name: "Remove “Acme CRM”?" });
    expect(within(dialog).getByText(/Removes the server from all clients on this account/)).toBeInTheDocument();
    expect(within(dialog).getByText(/deletes its credential on this device/)).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "Remove server" }));

    await waitFor(() => expect(mcpServerDeleteMock).toHaveBeenCalledWith("srv-1"));
    expect(onRegistryChanged).toHaveBeenCalledOnce();
  });

  it("leaves the server in place when the confirmation is declined", async () => {
    listResolves([registered()]);
    const user = userEvent.setup();

    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Remove Acme CRM" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    expect(mcpServerDeleteMock).not.toHaveBeenCalled();
    expect(screen.getByText("Acme CRM")).toBeInTheDocument();
  });

  it("turns a typed removal failure into a sentence instead of showing its code", async () => {
    listResolves([registered()]);
    mcpServerDeleteMock.mockRejectedValueOnce({ version: 1, code: "not_found" });
    const user = userEvent.setup();

    render(<McpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Remove Acme CRM" }));
    await user.click(screen.getByRole("button", { name: "Remove server" }));

    const alert = await screen.findByText("This server is no longer registered on this account. Refresh the list.");
    expect(alert).toBeInTheDocument();
    expect(screen.queryByText(/not_found/)).not.toBeInTheDocument();
  });

  it("offers the empty state only for the user's own servers, not the bundled one", async () => {
    listResolves([bundled]);

    render(<McpServersSection />);

    expect(await screen.findByText("No additional servers")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add an MCP server" })).not.toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Add server" })).toHaveLength(1);
  });

  it("reports a registry it cannot read rather than an empty list", async () => {
    mcpServerListMock.mockRejectedValueOnce({ version: 1, code: "service_unavailable" });

    render(<McpServersSection />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The server registry could not be reached. Try again in a moment.",
    );
    expect(screen.queryByText("No additional servers")).not.toBeInTheDocument();
  });

  it("marks a disabled server as listed but not connected", async () => {
    listResolves([registered({ enabled: false })]);

    render(<McpServersSection />);

    expect(await screen.findByText("Disabled")).toBeInTheDocument();
  });

  it("shows the empty state as one line of text rather than a panel", async () => {
    listResolves([bundled]);

    const { container } = render(<McpServersSection />);

    expect(await screen.findByText("No additional servers")).toBeInTheDocument();
    expect(screen.getByText("Add one to connect more tools.")).toBeInTheDocument();
    expect(container.querySelector(".settings-group__empty")).toBeNull();
  });

  it("shows one status badge for a healthy signed-in server", async () => {
    listResolves([registered({ auth_type: "oauth", sign_in: "signed_in" })]);

    render(<McpServersSection />);

    expect(await screen.findByText("Signed in on this device")).toBeInTheDocument();
    expect(screen.queryByText("Enabled")).not.toBeInTheDocument();
    expect(screen.queryByText("Not in use")).not.toBeInTheDocument();
    expect(screen.queryByText("Disabled")).not.toBeInTheDocument();
  });

  it("shows Enabled for a healthy server that needs no sign-in", async () => {
    listResolves([registered()]);

    render(<McpServersSection />);

    expect(await screen.findByText("Enabled")).toBeInTheDocument();
    expect(screen.queryByText("Disabled")).not.toBeInTheDocument();
  });

  it("shows Not in use instead of the sign-in badge when a server is unavailable", async () => {
    listResolves([registered({ auth_type: "oauth", sign_in: "signed_in", unavailable: "invalid_url" })]);

    render(<McpServersSection />);

    expect(await screen.findByText("Not in use")).toBeInTheDocument();
    expect(screen.queryByText("Signed in on this device")).not.toBeInTheDocument();
    expect(screen.queryByText("Enabled")).not.toBeInTheDocument();
  });

  it("shows Disabled alongside the status for a disabled server", async () => {
    listResolves([registered({ enabled: false, auth_type: "oauth", sign_in: "signed_in" })]);

    render(<McpServersSection />);

    expect(await screen.findByText("Disabled")).toBeInTheDocument();
    expect(screen.getByText("Signed in on this device")).toBeInTheDocument();
    expect(screen.queryByText("Enabled")).not.toBeInTheDocument();
  });

  it("keeps the one-line summary visible with details closed and open", async () => {
    listResolves([registered({ permission_mode: "allow" })]);
    const user = userEvent.setup();

    render(<McpServersSection />);

    expect(await screen.findByText("Streamable HTTP · Bearer token · Allow")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show details" }));
    expect(screen.getByText("Streamable HTTP · Bearer token · Allow")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Hide details" })).toBeInTheDocument();
  });

  it("shows labelled Edit and Remove actions, named for their server, and runs their handlers", async () => {
    listResolves([registered()]);
    const user = userEvent.setup();

    render(<McpServersSection />);

    const edit = await screen.findByRole("button", { name: "Edit Acme CRM" });
    expect(edit).toHaveTextContent("Edit");
    expect(screen.getByRole("button", { name: "Remove Acme CRM" })).toHaveTextContent("Remove");
    expect(screen.getByRole("button", { name: "Refresh" })).toHaveAttribute("data-settings-variant", "secondary");
    await user.click(edit);
    expect(screen.getByRole("dialog", { name: "Edit MCP server" })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Edit MCP server" })).not.toBeInTheDocument());

    await user.click(screen.getByRole("button", { name: "Remove Acme CRM" }));
    expect(screen.getByRole("alertdialog", { name: "Remove “Acme CRM”?" })).toBeInTheDocument();
    expect(mcpServerDeleteMock).not.toHaveBeenCalled();
  });

  it("shows the bundled endpoint and one caption without a definition list", async () => {
    listResolves([bundled]);

    const { container } = render(<McpServersSection />);

    expect(await screen.findByText("https://tenant.example/mcp")).toBeInTheDocument();
    expect(screen.getByText("Streamable HTTP · Browser sign-in")).toBeInTheDocument();
    expect(container.querySelector("dl")).toBeNull();
    expect(screen.getByRole("button", { name: "Remove Workspace MCP server" })).toHaveTextContent("Remove");
  });
});

// Every arm of the contract union is covered above; this pins that fact so a new
// arm cannot be added without a test for the prose it needs.
const COVERED_REASONS: Record<McpServerUnavailableReason, true> = {
  unsupported_transport: true,
  unsupported_auth_method: true,
  needs_sign_in: true,
  missing_device_credential: true,
  invalid_url: true,
};

describe("unavailable reasons", () => {
  it("covers every reason the contract can report", () => {
    expect(Object.keys(COVERED_REASONS)).toHaveLength(5);
  });
});
