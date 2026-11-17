import React from "react";
import { Alert } from "../../ui/Alert";
import { Field, Input } from "../../ui/Field";
import { Modal } from "../../ui/Modal";
import { SettingsButton } from "../settings/SettingsControls";

interface McpClientDetailsDialogProps {
  /** The row needing client details, or `null` when none does. */
  server: { id: string; label: string } | null;
  error?: string | null;
  busy: boolean;
  onCancel: () => void;
  onSubmit: (clientId: string, clientSecret: string | null) => void;
}

/**
 * Asks for the client ID an operator issued, for an authorization server that
 * will not issue one automatically.
 *
 * Shown only after a sign-in has already been attempted without it. That order
 * matters: most MCP servers register a client on request, and asking every user
 * for a value they usually do not need would make the common case look harder
 * than it is.
 */
export function McpClientDetailsDialog({ server, busy, error, onCancel, onSubmit }: McpClientDetailsDialogProps) {
  const fieldId = React.useId();
  const [clientId, setClientId] = React.useState("");
  const [clientSecret, setClientSecret] = React.useState("");

  // Cleared whenever the dialog changes rows, so a value typed for one server
  // cannot be submitted against another.
  React.useEffect(() => {
    setClientId("");
    setClientSecret("");
  }, [server?.id]);

  const trimmedId = clientId.trim();

  return (
    <Modal
      footer={
        <>
          <SettingsButton disabled={busy} onClick={onCancel}>
            Cancel
          </SettingsButton>
          <SettingsButton
            disabled={trimmedId.length === 0}
            loading={busy}
            onClick={() => onSubmit(trimmedId, clientSecret.trim() || null)}
            variant="primary"
          >
            Sign in
          </SettingsButton>
        </>
      }
      onClose={onCancel}
      open={server != null}
      dismissible={!busy}
      title={server ? `Sign in to “${server.label}”` : "Sign in"}
    >
      <div className="flex flex-col gap-4">
        {error ? (
          <Alert title="Sign-in failed" tone="danger">
            {error}
          </Alert>
        ) : null}
        <Alert title="Client registration required" tone="info">
          Enter the client ID supplied by the server administrator.
        </Alert>

        <Field
          hint="Sometimes called an application ID or connector ID."
          htmlFor={`${fieldId}-client-id`}
          label="Client ID"
        >
          <Input
            autoComplete="off"
            disabled={busy}
            onChange={(event) => setClientId(event.target.value)}
            spellCheck={false}
            value={clientId}
          />
        </Field>
        <Field
          hint="Optional. Only enter a secret if the server administrator supplied one for this client ID."
          htmlFor={`${fieldId}-client-secret`}
          label="Client secret"
        >
          <Input
            autoComplete="new-password"
            disabled={busy}
            onChange={(event) => setClientSecret(event.target.value)}
            spellCheck={false}
            type="password"
            value={clientSecret}
          />
        </Field>
      </div>
    </Modal>
  );
}
