import React from "react";
import type { McpPermissionMode, McpServerInput, McpServerView } from "../../lib/generated/contracts";
import { mcpServerCreate, mcpServerUpdate } from "../../lib/native";
import { Alert } from "../../ui/Alert";
import { Field, Input, Textarea } from "../../ui/Field";
import { Modal } from "../../ui/Modal";
import { Radio, RadioGroup } from "../../ui/Radio";
import { SettingsButton } from "./SettingsControls";
import { MCP_PERMISSION_MODE, describeMcpServerError } from "./settingsCopy";

const PERMISSION_MODES: readonly McpPermissionMode[] = ["ask", "allow", "deny"];

/**
 * What the Client secret field shows while this device holds a sealed one.
 *
 * A stored secret is never sent back to the surface, so the field cannot be
 * seeded with it. An empty box over a saved value reads as "it was wiped" --
 * which is how this was reported -- so the placeholder stands for the value the
 * hint describes. It is a placeholder and not a value: the field is still empty,
 * and empty still means "keep what is sealed".
 */
const SEALED_SECRET_PLACEHOLDER = "••••••••••••••••";

export interface McpServerSheetProps {
  open: boolean;
  /** The row to edit, or `null` to register a new server. */
  server: McpServerView | null;
  onClose: () => void;
  /**
   * The saved row, as the backend now reports it. Everything the sheet
   * collected is already stored by the time this runs -- including the
   * server's client details -- so a caller that starts sign-in needs to pass
   * nothing along: `mcp_server_sign_in` reads the stored client itself.
   */
  onSaved: (server: McpServerView) => void;
}

/**
 * Registers and edits one MCP server.
 *
 * Every server this app registers authenticates by browser sign-in, so there is
 * no method to choose: the tenant's servers sit behind the same identity
 * provider the app itself signs in to. A row DA reports with another method is
 * still listed and described by `McpServersSection`, but it is not something
 * this sheet can create.
 */
export function McpServerSheet({ open, server, onClose, onSaved }: McpServerSheetProps) {
  const [name, setName] = React.useState("");
  const [displayName, setDisplayName] = React.useState("");
  const [description, setDescription] = React.useState("");
  const [url, setUrl] = React.useState("");
  const [clientId, setClientId] = React.useState("");
  const [clientSecret, setClientSecret] = React.useState("");
  // `allow` rather than `ask`: every server registered here is the tenant's own,
  // and a prompt on each read was approval fatigue rather than a decision. An
  // edit still shows what the row actually holds.
  const [permissionMode, setPermissionMode] = React.useState<McpPermissionMode>("allow");
  const [submitting, setSubmitting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const operationRef = React.useRef(0);
  const submittingRef = React.useRef(false);

  // Re-seeded from the row on every open, so a cancelled edit leaves nothing
  // behind -- least of all a secret typed into the previous attempt.
  //
  // The client ID is the one field seeded from stored state rather than blanked.
  // It is not confidential, and a blank field over a value the operator already
  // supplied reads as data loss. It is required, so it is replaced rather than
  // cleared: the backend still reads a blank ID as "remove the stored client",
  // and this sheet no longer submits one.
  React.useEffect(() => {
    operationRef.current += 1;
    setClientSecret("");
    if (!open) {
      submittingRef.current = false;
      setClientId("");
      return;
    }
    setName(server?.name ?? "");
    setDisplayName(server?.display_name ?? "");
    setDescription(server?.description ?? "");
    setUrl(server?.url ?? "");
    setClientId(server?.oauth_client_id ?? "");
    setPermissionMode(server?.permission_mode ?? "allow");
    setSubmitting(false);
    submittingRef.current = false;
    setError(null);
  }, [open, server]);

  const editing = server != null;
  // What the row reports this device holds.
  const storedClientId = server?.oauth_client_id ?? "";
  const deviceHasClientSecret = editing && server.oauth_client_confidential;

  const trimmedName = name.trim();
  const trimmedUrl = url.trim();
  const trimmedClientId = clientId.trim();
  // A sealed value is kept by leaving the field blank, so it counts as a
  // secret already present -- but only for the client id it was issued
  // against. Retyping the id means the sealed one no longer applies.
  const clientSecretRetained = deviceHasClientSecret && trimmedClientId === storedClientId;
  // Required, and a sealed secret is how it is met on an edit: the field cannot
  // be seeded, so demanding a typed value here would make every edit re-enter a
  // secret this device already holds.
  const clientSecretSupplied = clientSecret.trim().length > 0 || clientSecretRetained;
  const canSubmit =
    trimmedName.length > 0 &&
    trimmedUrl.length > 0 &&
    trimmedClientId.length > 0 &&
    clientSecretSupplied &&
    !submitting;

  const buildInput = (): McpServerInput => ({
    name: trimmedName,
    display_name: displayName.trim() || null,
    description: description.trim() || null,
    url: trimmedUrl,
    // The only method this app registers. Stated here rather than chosen by the
    // reader: a server reached from this app is a server behind the tenant's
    // identity provider, and every other method was a way to get that wrong.
    auth_type: "oauth",
    header_name: null,
    permission_mode: permissionMode,
    enabled: true,
  });

  const submit = async () => {
    if (!canSubmit || submittingRef.current) return;
    const operation = operationRef.current;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    // Declarative, not a patch: the field contents *are* the client to keep. A
    // blank secret beside an unchanged ID keeps what is sealed, which the backend
    // resolves -- it is the only field this sheet cannot seed.
    const oauth = { clientId: trimmedClientId || null, clientSecret: clientSecret.trim() || null };
    try {
      const saved = editing
        ? await mcpServerUpdate(server.id, buildInput(), oauth)
        : await mcpServerCreate(buildInput(), oauth);
      if (operationRef.current !== operation) return;
      if (editing && saved.id !== server.id) {
        setError("The saved server did not match the server being edited. Nothing on this sheet was replaced.");
        return;
      }
      setClientSecret("");
      onSaved(saved);
    } catch (cause: unknown) {
      if (operationRef.current !== operation) return;
      setError(
        describeMcpServerError(
          cause,
          editing ? "This server could not be updated." : "This server could not be added.",
        ),
      );
    } finally {
      if (operationRef.current === operation) {
        submittingRef.current = false;
        setSubmitting(false);
      }
    }
  };

  return (
    <Modal
      footer={
        <>
          <SettingsButton disabled={submitting} onClick={onClose}>
            Cancel
          </SettingsButton>
          <SettingsButton disabled={!canSubmit} loading={submitting} onClick={() => void submit()} variant="primary">
            {editing ? "Save changes" : "Add server"}
          </SettingsButton>
        </>
      }
      onClose={onClose}
      open={open}
      dismissible={!submitting}
      size="lg"
      title={editing ? "Edit MCP server" : "Add MCP server"}
    >
      <div className="flex flex-col gap-4">
        <Field
          hint={
            editing
              ? "The name identifies the server's tools and cannot be changed."
              : "A unique account name used to identify the server's tools."
          }
          htmlFor="mcp-server-name"
          label="Name"
          required
        >
          <Input
            autoComplete="off"
            disabled={editing || submitting}
            onChange={(event) => setName(event.target.value)}
            placeholder="acme-crm"
            value={name}
          />
        </Field>

        <Field hint="Shown in this list instead of the name." htmlFor="mcp-server-display-name" label="Display name">
          <Input
            autoComplete="off"
            disabled={submitting}
            onChange={(event) => setDisplayName(event.target.value)}
            placeholder="Acme CRM"
            value={displayName}
          />
        </Field>

        <Field htmlFor="mcp-server-description" label="Description">
          <Textarea
            disabled={submitting}
            onChange={(event) => setDescription(event.target.value)}
            rows={2}
            value={description}
          />
        </Field>

        <Field
          hint="An https address serving MCP over Streamable HTTP."
          htmlFor="mcp-server-url"
          label="Server address"
          required
        >
          <Input
            autoComplete="off"
            disabled={submitting}
            onChange={(event) => setUrl(event.target.value)}
            placeholder="https://mcp.example.com/mcp"
            spellCheck={false}
            value={url}
          />
        </Field>

        <Alert title="Browser sign-in" tone="info">
          Add the server and complete sign-in in the browser. The server supplies its sign-in address automatically.
        </Alert>

        <Field
          hint={
            storedClientId
              ? "Saved on this device. Change it to use a different client."
              : "The client ID supplied by the server administrator. Saved on this device."
          }
          htmlFor="mcp-server-client-id"
          label="Client ID"
          required
        >
          <Input
            autoComplete="off"
            disabled={submitting}
            onChange={(event) => setClientId(event.target.value)}
            spellCheck={false}
            value={clientId}
          />
        </Field>

        <Field
          hint={
            clientSecretRetained
              ? "A secret is saved on this device for this client ID. Leave blank to keep it, or type a new one to replace it."
              : deviceHasClientSecret
                ? "The saved secret belongs to the previous client ID and will be replaced. Enter the one issued for this client ID."
                : "The secret issued for this client ID. Saved on this device."
          }
          htmlFor="mcp-server-client-secret"
          label="Client secret"
          // Met by the sealed value while the client ID it belongs to is
          // unchanged, so an edit is not asked for what it cannot show.
          required={!clientSecretRetained}
        >
          <Input
            autoComplete="new-password"
            disabled={submitting}
            onChange={(event) => setClientSecret(event.target.value)}
            // Only where one is actually held: a placeholder on a field with
            // nothing behind it would claim a secret this device does not have.
            placeholder={clientSecretRetained ? SEALED_SECRET_PLACEHOLDER : undefined}
            spellCheck={false}
            type="password"
            value={clientSecret}
          />
        </Field>

        <RadioGroup
          label="Tool approvals"
          name="mcp-server-permission-mode"
          onValueChange={(next) => setPermissionMode(next as McpPermissionMode)}
          value={permissionMode}
        >
          {PERMISSION_MODES.map((mode) => (
            <Radio disabled={submitting} key={mode} value={mode}>
              <span className="flex flex-col">
                <span className="text-foreground">{MCP_PERMISSION_MODE[mode].label}</span>
                <span className="type-caption text-muted">{MCP_PERMISSION_MODE[mode].meaning}</span>
              </span>
            </Radio>
          ))}
        </RadioGroup>

        {error ? (
          <p className="type-caption text-danger" role="alert">
            {error}
          </p>
        ) : null}
      </div>
    </Modal>
  );
}
