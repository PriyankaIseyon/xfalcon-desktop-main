import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { McpServerView } from "../../lib/generated/contracts";
import { McpServerSheet } from "./McpServerSheet";

const mcpServerCreateMock = vi.hoisted(() => vi.fn());
const mcpServerUpdateMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  mcpServerCreate: mcpServerCreateMock,
  mcpServerUpdate: mcpServerUpdateMock,
}));

const existing: McpServerView = {
  version: 1,
  id: "srv-1",
  name: "acme-crm",
  display_name: "Acme CRM",
  description: "Customer records",
  transport: "http",
  url: "https://mcp.acme.example/mcp",
  auth_type: "oauth",
  permission_mode: "ask",
  enabled: true,
  builtin: false,
  slug: "acme-crm",
  unavailable: null,
  sign_in: "signed_in",
  oauth_client_id: null,
  oauth_client_confidential: false,
};

/** The same row, with a client sealed on this device. */
const held: McpServerView = {
  ...existing,
  oauth_client_id: "issued-client",
  oauth_client_confidential: true,
};

afterEach(() => {
  cleanup();
  mcpServerCreateMock.mockReset();
  mcpServerUpdateMock.mockReset();
});

/**
 * `Modal` moves focus into its panel on an animation frame. A test that types
 * before that lands races it and loses the keystrokes that arrive after, so any
 * test entering text waits for the focus move first -- which also pins that the
 * sheet does take focus when it opens.
 */
async function modalHasTakenFocus() {
  await waitFor(() => expect(screen.getByRole("button", { name: "Close" })).toHaveFocus());
}

describe("McpServerSheet", () => {
  it("registers browser sign-in only, with no method to choose and nothing to test", () => {
    render(<McpServerSheet open server={null} onClose={vi.fn()} onSaved={vi.fn()} />);

    expect(screen.queryByRole("radiogroup", { name: "Authentication" })).not.toBeInTheDocument();
    expect(screen.getByText("Browser sign-in")).toBeInTheDocument();
    // The methods that are gone, and the fields that only they needed.
    expect(screen.queryByLabelText("Header name")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Header value")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Token")).not.toBeInTheDocument();
    // Testing needed a saved, unedited row, so it was unreachable from the one
    // flow that registers a server. Sign-in reports the connection instead.
    expect(screen.queryByRole("button", { name: "Test connection" })).not.toBeInTheDocument();
    expect(screen.queryByText(/before testing/)).not.toBeInTheDocument();
  });

  it("registers an OAuth server from the four values it requires", async () => {
    const user = userEvent.setup();
    const onSaved = vi.fn();
    mcpServerCreateMock.mockResolvedValueOnce(existing);
    render(<McpServerSheet open server={null} onClose={vi.fn()} onSaved={onSaved} />);
    await modalHasTakenFocus();

    await user.type(screen.getByLabelText(/^Name/), "acme-crm");
    await user.type(screen.getByLabelText("Display name"), "Acme CRM");
    await user.type(screen.getByLabelText(/^Server address/), "https://mcp.acme.example/mcp");
    await user.type(screen.getByLabelText(/^Client ID/), "issued-client");
    await user.type(screen.getByLabelText(/^Client secret/), "issued-secret");
    await user.click(screen.getByRole("button", { name: "Add server" }));

    await waitFor(() =>
      expect(mcpServerCreateMock).toHaveBeenCalledWith(
        {
          name: "acme-crm",
          display_name: "Acme CRM",
          description: null,
          url: "https://mcp.acme.example/mcp",
          auth_type: "oauth",
          header_name: null,
          // Allow, not Ask: these servers are the tenant's own.
          permission_mode: "allow",
          enabled: true,
        },
        { clientId: "issued-client", clientSecret: "issued-secret" },
      ),
    );
    expect(onSaved).toHaveBeenCalledWith(existing);
  });

  it("saves an edited server as connected without an opt-out control", async () => {
    const user = userEvent.setup();
    mcpServerUpdateMock.mockResolvedValueOnce(held);
    render(<McpServerSheet open server={{ ...held, enabled: false }} onClose={vi.fn()} onSaved={vi.fn()} />);
    await modalHasTakenFocus();

    expect(screen.queryByRole("switch", { name: "Connect this server during chat" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() =>
      expect(mcpServerUpdateMock).toHaveBeenCalledWith(held.id, expect.objectContaining({ enabled: true }), {
        clientId: "issued-client",
        clientSecret: null,
      }),
    );
  });

  it("marks every value it requires, and refuses to save until each is supplied", async () => {
    const user = userEvent.setup();
    render(<McpServerSheet open server={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await modalHasTakenFocus();

    // The required marker is on the label, so it reaches assistive tech as well
    // as the eye -- a red asterisk alone says nothing to a screen reader.
    for (const label of ["Name", "Server address", "Client ID", "Client secret"]) {
      expect(screen.getByText(label, { selector: "label" })).toHaveTextContent(", required");
    }
    expect(screen.getByText("Display name", { selector: "label" })).not.toHaveTextContent(", required");

    const add = screen.getByRole("button", { name: "Add server" });
    expect(add).toBeDisabled();
    await user.type(screen.getByLabelText(/^Name/), "acme-crm");
    expect(add).toBeDisabled();
    await user.type(screen.getByLabelText(/^Server address/), "https://mcp.acme.example/mcp");
    expect(add).toBeDisabled();
    await user.type(screen.getByLabelText(/^Client ID/), "issued-client");
    expect(add).toBeDisabled();
    await user.type(screen.getByLabelText(/^Client secret/), "issued-secret");
    expect(add).toBeEnabled();
  });

  it("saves client details on this device and keeps them out of the registry payload", async () => {
    const user = userEvent.setup();
    const onSaved = vi.fn();
    mcpServerCreateMock.mockResolvedValueOnce(existing);
    render(<McpServerSheet open server={null} onClose={vi.fn()} onSaved={onSaved} />);
    await modalHasTakenFocus();

    await user.type(screen.getByLabelText(/^Name/), "acme-crm");
    await user.type(screen.getByLabelText(/^Server address/), "https://mcp.acme.example/mcp");
    await user.type(screen.getByLabelText(/^Client secret/), "client-secret-value");
    // A secret with no client to identify is not a client.
    expect(screen.getByRole("button", { name: "Add server" })).toBeDisabled();
    await user.type(screen.getByLabelText(/^Client ID/), "  issued-client  ");
    await user.click(screen.getByRole("button", { name: "Add server" }));

    await waitFor(() => expect(onSaved).toHaveBeenCalledWith(existing));
    // The pair reaches the device store as its own argument -- saving is what
    // persists it, not a sign-in that may never be completed.
    expect(mcpServerCreateMock).toHaveBeenCalledWith(expect.objectContaining({ auth_type: "oauth" }), {
      clientId: "issued-client",
      clientSecret: "client-secret-value",
    });
    // ...and never inside the definition written to the shared registry.
    const [input] = mcpServerCreateMock.mock.calls[0];
    expect(JSON.stringify(input)).not.toContain("client-secret-value");
    expect(JSON.stringify(input)).not.toContain("issued-client");
    expect(screen.getByLabelText(/^Client secret/)).toHaveValue("");
  });

  it("shows the client ID this device already holds, and states the secret it cannot show", async () => {
    const user = userEvent.setup();
    mcpServerUpdateMock.mockResolvedValueOnce(held);
    render(<McpServerSheet open server={held} onClose={vi.fn()} onSaved={vi.fn()} />);
    await modalHasTakenFocus();

    // The regression this covers: the field opened blank over a stored value,
    // so every edit read as "it never saved".
    expect(screen.getByLabelText(/^Client ID/)).toHaveValue("issued-client");
    const secret = screen.getByLabelText(/^Client secret/);
    expect(secret).toHaveValue("");
    // A sealed secret cannot be seeded, so the field says so twice: the
    // placeholder stands in for it, and the hint explains what blank means.
    expect(secret).toHaveAttribute("placeholder", "••••••••••••••••");
    expect(screen.getByText(/A secret is saved on this device for this client ID/)).toBeInTheDocument();

    await user.type(screen.getByLabelText("Description"), "!");
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() =>
      expect(mcpServerUpdateMock).toHaveBeenCalledWith(held.id, expect.anything(), {
        clientId: "issued-client",
        clientSecret: null,
      }),
    );
  });

  it("never claims a saved secret for a client this device holds none for", () => {
    render(
      <McpServerSheet
        open
        server={{ ...held, oauth_client_confidential: false }}
        onClose={vi.fn()}
        onSaved={vi.fn()}
      />,
    );

    expect(screen.getByLabelText(/^Client secret/)).not.toHaveAttribute("placeholder");
    expect(screen.queryByText(/A secret is saved on this device/)).not.toBeInTheDocument();
  });

  it("warns that a saved secret belongs to the client ID being replaced", async () => {
    const user = userEvent.setup();
    render(<McpServerSheet open server={held} onClose={vi.fn()} onSaved={vi.fn()} />);
    await modalHasTakenFocus();

    await user.clear(screen.getByLabelText(/^Client ID/));
    await user.type(screen.getByLabelText(/^Client ID/), "reissued-client");
    expect(
      screen.getByText(/The saved secret belongs to the previous client ID and will be replaced/),
    ).toBeInTheDocument();
    // The placeholder goes with it: that secret no longer applies to this ID.
    expect(screen.getByLabelText(/^Client secret/)).not.toHaveAttribute("placeholder");
  });

  it("does not submit a cleared client ID, because the field is required", async () => {
    const user = userEvent.setup();
    render(<McpServerSheet open server={held} onClose={vi.fn()} onSaved={vi.fn()} />);
    await modalHasTakenFocus();

    await user.clear(screen.getByLabelText(/^Client ID/));
    expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();

    // Replacing it is the supported move, and the secret goes with it.
    await user.type(screen.getByLabelText(/^Client ID/), "reissued-client");
    expect(screen.getByRole("button", { name: "Save changes" })).toBeDisabled();
    await user.type(screen.getByLabelText(/^Client secret/), "reissued-secret");
    expect(screen.getByRole("button", { name: "Save changes" })).toBeEnabled();
    expect(mcpServerUpdateMock).not.toHaveBeenCalled();
  });

  it("never carries a typed secret across a reopen", async () => {
    const user = userEvent.setup();
    const props = { server: null, onClose: vi.fn(), onSaved: vi.fn() };
    const view = render(<McpServerSheet {...props} open />);
    await modalHasTakenFocus();

    await user.type(screen.getByLabelText(/^Client ID/), "client-one");
    await user.type(screen.getByLabelText(/^Client secret/), "secret-one");
    view.rerender(<McpServerSheet {...props} open={false} />);
    view.rerender(<McpServerSheet {...props} open />);
    await modalHasTakenFocus();

    expect(screen.getByLabelText(/^Client ID/)).toHaveValue("");
    expect(screen.getByLabelText(/^Client secret/)).toHaveValue("");
  });

  it("states what each approval mode does before the choice is made", () => {
    render(<McpServerSheet open server={null} onClose={vi.fn()} onSaved={vi.fn()} />);

    const approvals = screen.getByRole("radiogroup", { name: "Tool approvals" });
    expect(within(approvals).getByRole("radio", { name: /^Allow/ })).toBeChecked();
    expect(within(approvals).getByRole("radio", { name: /^Ask/ })).not.toBeChecked();
    expect(within(approvals).getByText(/Every tool call requires approval/)).toBeInTheDocument();
    expect(within(approvals).getByText(/Tools that can change data require approval/)).toBeInTheDocument();
    expect(within(approvals).getByText(/All tools from this server are disabled/)).toBeInTheDocument();
  });

  it("does not invite an edit to a name that cannot change", async () => {
    mcpServerUpdateMock.mockResolvedValueOnce(held);
    const user = userEvent.setup();

    render(<McpServerSheet open server={held} onClose={vi.fn()} onSaved={vi.fn()} />);
    await modalHasTakenFocus();

    expect(screen.getByLabelText(/^Name/)).toBeDisabled();
    expect(screen.getByText(/cannot be changed/)).toBeInTheDocument();

    await user.clear(screen.getByLabelText("Description"));
    await user.type(screen.getByLabelText("Description"), "Updated notes");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(mcpServerUpdateMock).toHaveBeenCalledWith(
        "srv-1",
        expect.objectContaining({ description: "Updated notes", auth_type: "oauth" }),
        { clientId: "issued-client", clientSecret: null },
      ),
    );
  });

  it("refuses a saved row that does not match the server being edited", async () => {
    mcpServerUpdateMock.mockResolvedValueOnce({ ...held, id: "srv-other" });
    const onSaved = vi.fn();
    const user = userEvent.setup();

    render(<McpServerSheet open server={held} onClose={vi.fn()} onSaved={onSaved} />);
    await modalHasTakenFocus();
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The saved server did not match the server being edited.",
    );
    expect(onSaved).not.toHaveBeenCalled();
  });

  it("turns a typed save failure into a sentence instead of showing its code", async () => {
    mcpServerCreateMock.mockRejectedValueOnce({ version: 1, code: "url_rejected" });
    const user = userEvent.setup();

    render(<McpServerSheet open server={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await modalHasTakenFocus();
    await user.type(screen.getByLabelText(/^Name/), "acme-crm");
    await user.type(screen.getByLabelText(/^Server address/), "https://10.0.0.5/mcp");
    await user.type(screen.getByLabelText(/^Client ID/), "issued-client");
    await user.type(screen.getByLabelText(/^Client secret/), "issued-secret");
    await user.click(screen.getByRole("button", { name: "Add server" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("A public HTTPS address is required.");
    expect(screen.queryByText(/url_rejected/)).not.toBeInTheDocument();
  });
});
