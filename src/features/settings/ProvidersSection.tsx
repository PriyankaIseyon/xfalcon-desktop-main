import React from "react";
import { Plus, RotateCcw } from "lucide-react";
import type { ProviderCredentialView, ProviderId } from "../../lib/generated/contracts";
import { credentialCreate, credentialDelete, credentialList, credentialReplace } from "../../lib/native";
import { Alert } from "../../ui/Alert";
import { AlertDialog } from "../../ui/AlertDialog";
import { Badge } from "../../ui/Badge";
import { Field, Input } from "../../ui/Field";
import { List, ListItem } from "../../ui/List";
import { Modal } from "../../ui/Modal";
import { Radio, RadioGroup } from "../../ui/Radio";
import { Spinner } from "../../ui/Spinner";
import { Switch } from "../../ui/Switch";
import { toast } from "../../ui/toastStore";
import { SettingsButton, SettingsRemoveButton } from "./SettingsControls";
import { SettingsActions, SettingsGroup, SettingsGroupHeader } from "./SettingsLayout";
import {
  CREDENTIAL_NEEDS_REENTRY,
  CREDENTIAL_STATUS,
  PROVIDER_IDS,
  PROVIDER_KEY_HINT,
  PROVIDER_LABEL,
  describeCredentialError,
  credentialErrorCode,
} from "./settingsCopy";

type ListState =
  | { status: "loading" }
  | { status: "ready"; credentials: ProviderCredentialView[] }
  | { status: "error"; message: string };

export interface ProvidersSectionProps {
  /**
   * Called after a key is stored or removed. The shell uses it to re-read
   * `workspace_status`, which is what turns the composer on and off: adding the
   * first key is the moment chat becomes usable, and removing the last one is
   * the moment it stops.
   */
  onCredentialsChanged?: () => void;
}

export function ProvidersSection({ onCredentialsChanged }: ProvidersSectionProps) {
  const [state, setState] = React.useState<ListState>({ status: "loading" });
  const [reloadToken, setReloadToken] = React.useState(0);
  const [saveNotice, setSaveNotice] = React.useState<string | null>(null);
  const [addOpen, setAddOpen] = React.useState(false);
  const [reentering, setReentering] = React.useState<ProviderCredentialView | null>(null);
  const [pendingRemoval, setPendingRemoval] = React.useState<ProviderCredentialView | null>(null);
  const [removing, setRemoving] = React.useState(false);
  const [removeError, setRemoveError] = React.useState<string | null>(null);
  // The rows the confirmation was opened over. Held separately from `state`
  // because a failed removal reloads the list, and the dialog's wording must
  // not flicker to "last key" while that reload is in flight.
  const [removalList, setRemovalList] = React.useState<ProviderCredentialView[]>([]);

  React.useEffect(() => {
    let active = true;
    credentialList().then(
      (credentials) => {
        if (active) setState({ status: "ready", credentials });
      },
      (cause: unknown) => {
        if (active) {
          setState({
            status: "error",
            message: describeCredentialError(cause, "Provider keys could not be loaded."),
          });
        }
      },
    );
    return () => {
      active = false;
    };
  }, [reloadToken]);

  const reload = React.useCallback(() => {
    setState({ status: "loading" });
    setReloadToken((token) => token + 1);
  }, []);

  const refreshAfterUnconfirmedSave = (message: string) => {
    setAddOpen(false);
    setReentering(null);
    setSaveNotice(message);
    reload();
    onCredentialsChanged?.();
  };

  const remove = async (credential: ProviderCredentialView) => {
    setRemoving(true);
    setRemoveError(null);
    try {
      await credentialDelete(credential.id);
      setPendingRemoval(null);
      toast.success(`Removed the provider key “${credential.display_name}”.`);
      reload();
      onCredentialsChanged?.();
    } catch (cause: unknown) {
      if (credentialErrorCode(cause) === "reconciliation_required") {
        setPendingRemoval(null);
        setSaveNotice("The removal could not be confirmed. Review the updated key list.");
        reload();
        onCredentialsChanged?.();
        return;
      }
      setRemoveError(describeCredentialError(cause, "The provider key could not be removed."));
      // The list may be what went stale -- another client can have changed it
      // -- so the rows behind the dialog are re-read whatever the cause.
      reload();
    } finally {
      setRemoving(false);
    }
  };

  const openAdd = () => {
    setAddOpen(true);
  };

  return (
    <SettingsGroup id="settings-providers" aria-labelledby="settings-providers-heading">
      <SettingsGroupHeader
        headingId="settings-providers-heading"
        title="Model providers"
        description="Hosted model keys are encrypted, shared across signed-in clients, and cannot be viewed."
      >
        {state.status === "ready" ? (
          <SettingsButton icon={<Plus />} onClick={openAdd}>
            Add provider key
          </SettingsButton>
        ) : null}
      </SettingsGroupHeader>

      {saveNotice ? (
        <Alert title="Review provider keys" tone="warn">
          {saveNotice}
        </Alert>
      ) : null}

      {state.status === "loading" ? (
        <div className="settings-group__state text-secondary">
          <Spinner decorative size={16} />
          <span>Loading provider keys…</span>
        </div>
      ) : null}

      {state.status === "error" ? (
        <Alert className="settings-group__state" title="Provider keys unavailable" tone="danger">
          <p>{state.message}</p>
          <SettingsButton className="mt-2" onClick={reload}>
            Try again
          </SettingsButton>
        </Alert>
      ) : null}

      {state.status === "ready" && state.credentials.length === 0 ? (
        <div className="settings-group__state text-secondary">
          <strong className="font-semibold text-foreground">No provider keys</strong>
          <span>Add one to enable chat.</span>
        </div>
      ) : null}

      {state.status === "ready" && state.credentials.length > 0 ? (
        <List className="flex flex-col">
          {state.credentials.map((credential) => {
            const status = CREDENTIAL_STATUS[credential.status];
            return (
              <ListItem key={credential.id}>
                <div className="settings-collection-item">
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div className="flex min-w-0 flex-col gap-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <strong className="type-body break-all font-semibold text-foreground">
                          {credential.display_name}
                        </strong>
                        {credential.is_default ? <Badge tone="accent">Default key</Badge> : null}
                        <Badge tone={status.tone}>{status.label}</Badge>
                      </div>
                      <p className="type-caption text-muted">
                        <span>{PROVIDER_LABEL[credential.provider]}</span>
                        {status.detail ? <span> · {status.detail}</span> : null}
                      </p>
                    </div>
                    <SettingsActions>
                      {CREDENTIAL_NEEDS_REENTRY.has(credential.status) ? (
                        <SettingsButton
                          icon={<RotateCcw />}
                          onClick={() => {
                            setReentering(credential);
                          }}
                        >
                          Re-enter key
                        </SettingsButton>
                      ) : null}
                      <SettingsRemoveButton
                        aria-label={`Remove ${credential.display_name}`}
                        disabled={removing && pendingRemoval?.id === credential.id}
                        onClick={() => {
                          setRemoveError(null);
                          setRemovalList(state.credentials);
                          setPendingRemoval(credential);
                        }}
                      />
                    </SettingsActions>
                  </div>
                </div>
              </ListItem>
            );
          })}
        </List>
      ) : null}

      <ReenterProviderKeyDialog
        credential={reentering}
        onRefresh={refreshAfterUnconfirmedSave}
        onClose={() => setReentering(null)}
        onReplaced={(credential) => {
          setReentering(null);
          setSaveNotice(null);
          toast.success(`“${credential.display_name}” is stored securely and ready to use.`);
          reload();
          onCredentialsChanged?.();
        }}
      />

      <AddProviderKeyDialog
        open={addOpen}
        credentials={state.status === "ready" ? state.credentials : []}
        onRefresh={refreshAfterUnconfirmedSave}
        onClose={() => setAddOpen(false)}
        onCreated={(credential) => {
          setAddOpen(false);
          setSaveNotice(null);
          toast.success(`Added the provider key “${credential.display_name}”.`);
          reload();
          onCredentialsChanged?.();
        }}
      />

      <AlertDialog
        busy={removing}
        confirmLabel="Remove key"
        description={pendingRemoval ? removalDescription(pendingRemoval, removalList) : ""}
        destructive
        error={removeError}
        open={pendingRemoval !== null}
        onCancel={() => {
          setPendingRemoval(null);
          setRemoveError(null);
        }}
        onConfirm={() => {
          if (pendingRemoval) void remove(pendingRemoval);
        }}
        title={pendingRemoval ? `Remove “${pendingRemoval.display_name}”?` : "Remove provider key?"}
      />
    </SettingsGroup>
  );
}

/** Says chat stops only when it does: when no other ready key for that
 * provider is left to take over. */
function removalDescription(credential: ProviderCredentialView, credentials: ProviderCredentialView[]): string {
  const provider = PROVIDER_LABEL[credential.provider];
  const scope = `“${credential.display_name}” will be deleted for every signed-in xFalcon client, not only this desktop.`;
  const othersReady = credentials.some(
    (other) => other.id !== credential.id && other.provider === credential.provider && other.status === "ready",
  );
  return othersReady
    ? `${scope} Chat with ${provider} keeps working with your other ${provider} key.`
    : `${scope} Chat with ${provider} stops until another ${provider} key is added.`;
}

interface AddProviderKeyDialogProps {
  open: boolean;
  credentials: ProviderCredentialView[];
  onRefresh: (message: string) => void;
  onClose: () => void;
  onCreated: (credential: ProviderCredentialView) => void;
}

function AddProviderKeyDialog({ open, credentials, onRefresh, onClose, onCreated }: AddProviderKeyDialogProps) {
  const [provider, setProvider] = React.useState<ProviderId>("openai");
  const [secret, setSecret] = React.useState("");
  const [isDefault, setIsDefault] = React.useState(true);
  const [submitting, setSubmitting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const submittingRef = React.useRef(false);

  // A reopened dialog must never still be holding the previous attempt's secret.
  React.useEffect(() => {
    if (!open) return;
    setProvider("openai");
    setSecret("");
    setIsDefault(true);
    setSubmitting(false);
    submittingRef.current = false;
    setError(null);
  }, [open]);

  const labels = new Set(
    credentials.filter((credential) => credential.provider === provider).map((credential) => credential.display_name),
  );
  let sequence = 1;
  while (labels.has(`${provider}_${sequence}`)) sequence += 1;
  const label = `${provider}_${sequence}`;
  const canSubmit = secret.trim().length > 0 && !submitting;

  const close = () => {
    if (submittingRef.current) return;
    setSecret("");
    setError(null);
    onClose();
  };

  const submit = async () => {
    if (!canSubmit || submittingRef.current) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const credential = await credentialCreate(provider, label, secret.trim(), isDefault);
      setSecret("");
      onCreated(credential);
    } catch (cause: unknown) {
      const code = credentialErrorCode(cause);
      if (code === "reconciliation_required" || code === "conflict") {
        setSecret("");
        onRefresh(
          code === "conflict"
            ? "The key list has changed. Review the updated list before adding another key."
            : "The save could not be confirmed. Review the updated key list before adding the key again.",
        );
        return;
      }
      setError(describeCredentialError(cause, "The provider key could not be added."));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  };

  return (
    <Modal
      footer={
        <>
          <SettingsButton disabled={submitting} onClick={close}>
            Cancel
          </SettingsButton>
          <SettingsButton disabled={!canSubmit} loading={submitting} onClick={() => void submit()} variant="primary">
            Save key
          </SettingsButton>
        </>
      }
      onClose={close}
      open={open}
      dismissible={!submitting}
      title="Add provider key"
    >
      <div className="flex flex-col gap-4">
        <RadioGroup
          label="Provider"
          name="provider-key-provider"
          onValueChange={(next) => setProvider(next as ProviderId)}
          value={provider}
        >
          {PROVIDER_IDS.map((id) => (
            <Radio key={id} value={id}>
              {PROVIDER_LABEL[id]}
            </Radio>
          ))}
        </RadioGroup>

        <p className="type-caption text-secondary">Key name: {label}</p>

        <Field
          hint={`${PROVIDER_KEY_HINT[provider]}. Transferred securely and encrypted before storage. Saved keys cannot be displayed.`}
          htmlFor="provider-key-secret"
          label="API key"
        >
          <Input
            autoComplete="off"
            disabled={submitting}
            onChange={(event) => setSecret(event.target.value)}
            spellCheck={false}
            type="password"
            value={secret}
          />
        </Field>

        <Switch
          checked={isDefault}
          id="provider-key-default"
          label="Default key for this provider"
          onChange={setIsDefault}
        />

        {error ? (
          <p className="type-caption text-danger" role="alert">
            {error}
          </p>
        ) : null}
      </div>
    </Modal>
  );
}

interface ReenterProviderKeyDialogProps {
  onRefresh: (message: string) => void;
  /** The row being re-sealed, or `null` when the dialog is closed. */
  credential: ProviderCredentialView | null;
  onClose: () => void;
  onReplaced: (credential: ProviderCredentialView) => void;
}

/**
 * Pastes a fresh secret into an existing row.
 *
 * Deliberately offers no provider or label field: both belong to the row, and
 * the backend reads them from it rather than from this dialog, so there is
 * nothing here that could pair a key with the wrong provider.
 */
function ReenterProviderKeyDialog({ credential, onRefresh, onClose, onReplaced }: ReenterProviderKeyDialogProps) {
  const [secret, setSecret] = React.useState("");
  const [submitting, setSubmitting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const submittingRef = React.useRef(false);

  // A reopened dialog must never still be holding the previous attempt's secret.
  React.useEffect(() => {
    if (!credential) return;
    setSecret("");
    setSubmitting(false);
    submittingRef.current = false;
    setError(null);
  }, [credential]);

  const canSubmit = secret.trim().length > 0 && !submitting;

  const close = () => {
    if (submittingRef.current) return;
    setSecret("");
    setError(null);
    onClose();
  };

  const submit = async () => {
    if (!canSubmit || !credential || submittingRef.current) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const replacedCredential = await credentialReplace(credential.id, secret.trim());
      setSecret("");
      onReplaced(replacedCredential);
    } catch (cause: unknown) {
      if (credentialErrorCode(cause) === "reconciliation_required") {
        setSecret("");
        onRefresh("The save could not be confirmed. Review the updated key list before re-entering the key.");
        return;
      }
      setError(describeCredentialError(cause, "The provider key could not be saved."));
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  };

  return (
    <Modal
      footer={
        <>
          <SettingsButton disabled={submitting} onClick={close}>
            Cancel
          </SettingsButton>
          <SettingsButton disabled={!canSubmit} loading={submitting} onClick={() => void submit()} variant="primary">
            Save key
          </SettingsButton>
        </>
      }
      onClose={close}
      open={credential !== null}
      dismissible={!submitting}
      title="Re-enter provider key"
    >
      <div className="flex flex-col gap-4">
        <p className="type-caption text-secondary">
          {credential ? PROVIDER_LABEL[credential.provider] : ""} · {credential?.display_name}
        </p>
        <Field
          hint={`${credential ? PROVIDER_KEY_HINT[credential.provider] : ""}. Replaces the existing key and stores it encrypted for signed-in xFalcon clients.`}
          htmlFor="provider-key-reentry-secret"
          label="API key"
        >
          <Input
            autoComplete="off"
            disabled={submitting}
            onChange={(event) => setSecret(event.target.value)}
            spellCheck={false}
            type="password"
            value={secret}
          />
        </Field>

        {error ? (
          <p className="type-caption text-danger" role="alert">
            {error}
          </p>
        ) : null}
      </div>
    </Modal>
  );
}
