import React from "react";
import { LogIn, LogOut, Pencil, Plus, RefreshCw, Server } from "lucide-react";
import type { McpServerView } from "../../lib/generated/contracts";
import {
  mcpServerDelete,
  mcpServerList,
  mcpServerRestoreBundled,
  mcpServerSignIn,
  mcpServerSignOut,
} from "../../lib/native";
import { Alert } from "../../ui/Alert";
import { AlertDialog } from "../../ui/AlertDialog";
import { Badge } from "../../ui/Badge";
import { List, ListItem } from "../../ui/List";
import { Spinner } from "../../ui/Spinner";
import { toast } from "../../ui/toastStore";
import { McpClientDetailsDialog } from "../mcp/McpClientDetailsDialog";
import { McpServerSheet } from "./McpServerSheet";
import { SettingsButton, SettingsRemoveButton } from "./SettingsControls";
import { SettingsActions, SettingsGroup, SettingsGroupHeader } from "./SettingsLayout";
import { settingsItemElementId } from "../../app/settingsCatalog";
import { HighlightedText, SearchRevealHost } from "../../lib/highlightedText";
import {
  MCP_AUTH_TYPE_LABEL,
  MCP_PERMISSION_MODE,
  MCP_SIGN_IN_STATUS,
  MCP_TRANSPORT_LABEL,
  MCP_UNAVAILABLE_DETAIL,
  MCP_UNAVAILABLE_EXPLANATION,
  describeMcpServerError,
  describeSignOut,
  describeTransportHandling,
  mcpServerErrorCode,
} from "./settingsCopy";

type ListState =
  { status: "loading" } | { status: "ready"; servers: McpServerView[] } | { status: "error"; message: string };

export interface McpServersSectionProps {
  revision?: number;
  /**
   * Called after any registry change. The shell uses it to re-read
   * `workspace_status`, whose MCP summary is derived from the same registry.
   */
  onRegistryChanged?: () => void;
}

export function McpServersSection({ onRegistryChanged, revision }: McpServersSectionProps) {
  const [state, setState] = React.useState<ListState>({ status: "loading" });
  const [reloadToken, setReloadToken] = React.useState(0);
  /** The row being edited, or `null` inside `{ server: null }` to register one. */
  const [sheet, setSheet] = React.useState<{ server: McpServerView | null } | null>(null);
  const [pendingDelete, setPendingDelete] = React.useState<McpServerView | null>(null);
  const [deleting, setDeleting] = React.useState(false);
  const [restoring, setRestoring] = React.useState(false);
  const [restoreError, setRestoreError] = React.useState<string | null>(null);
  const [deleteError, setDeleteError] = React.useState<string | null>(null);
  const [signInPending, setSignInPending] = React.useState<string | null>(null);
  const [signInError, setSignInError] = React.useState<RowError | null>(null);
  /**
   * The row whose authorization server would not issue a client, so the user has
   * to supply the one their operator issued. Set only after a sign-in has been
   * attempted without it -- the common case needs no input at all.
   */
  const [clientDetailsFor, setClientDetailsFor] = React.useState<McpServerView | null>(null);
  /**
   * The row awaiting a sign-out confirmation. Only a workspace-wide sign-in needs
   * one: it takes the grant away from every other client on the account, and an
   * action with that reach must not happen on a single unconfirmed click.
   */
  const [pendingSignOut, setPendingSignOut] = React.useState<McpServerView | null>(null);
  const [signOutError, setSignOutError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let active = true;
    mcpServerList().then(
      (result) => {
        if (active) setState({ status: "ready", servers: result.servers });
      },
      (cause: unknown) => {
        if (active) {
          setState({
            status: "error",
            message: describeMcpServerError(cause, "MCP servers could not be listed."),
          });
        }
      },
    );
    return () => {
      active = false;
    };
  }, [reloadToken, revision]);

  const reload = React.useCallback(() => {
    setState({ status: "loading" });
    setReloadToken((token) => token + 1);
  }, []);

  const confirmDelete = async () => {
    if (pendingDelete == null || deleting) return;
    setDeleting(true);
    setDeleteError(null);
    try {
      await mcpServerDelete(pendingDelete.id);
      toast.success(`Removed “${serverTitle(pendingDelete)}”.`);
      setPendingDelete(null);
      reload();
      onRegistryChanged?.();
    } catch (cause: unknown) {
      setDeleteError(describeMcpServerError(cause, "This server could not be removed."));
    } finally {
      setDeleting(false);
    }
  };

  const restoreBundled = async () => {
    if (restoring) return;
    setRestoring(true);
    setRestoreError(null);
    try {
      await mcpServerRestoreBundled();
      reload();
      onRegistryChanged?.();
      toast.success("Restored the workspace MCP server.");
    } catch (cause: unknown) {
      setRestoreError(describeMcpServerError(cause, "The workspace MCP server could not be restored."));
    } finally {
      setRestoring(false);
    }
  };

  /** Replaces one row in place after a sign-in or sign-out, which returns it. */
  const replaceRow = React.useCallback((updated: McpServerView) => {
    setState((current) =>
      current.status === "ready"
        ? {
            status: "ready",
            servers: current.servers.map((server) => (server.id === updated.id ? updated : server)),
          }
        : current,
    );
  }, []);

  /**
   * Re-reads the registry without clearing the list first.
   *
   * Unlike `reload`, this keeps the rows and any per-row failure on screen while
   * it runs, so it can be used on a path that has just reported an error. A read
   * that fails changes nothing: the rows already on screen are still the best
   * answer available, and replacing them with a panel-wide error would discard
   * the message explaining what the user just tried.
   */
  const refreshRows = React.useCallback(async () => {
    try {
      const result = await mcpServerList();
      setState((current) => (current.status === "ready" ? { status: "ready", servers: result.servers } : current));
    } catch {
      // Intentionally silent -- see above.
    }
  }, []);

  const signIn = async (server: McpServerView, clientId: string | null, clientSecret: string | null = null) => {
    if (signInPending != null) return;
    setSignInPending(server.id);
    setSignInError(null);
    try {
      replaceRow(await mcpServerSignIn(server.id, clientId, clientSecret));
      setClientDetailsFor(null);
      toast.success(`Signed in to “${serverTitle(server)}” on this device.`);
      onRegistryChanged?.();
    } catch (cause: unknown) {
      const message = describeMcpServerError(cause, "This server could not be signed in to.");
      // A sign-in that was given client details has already sealed them, even
      // though the flow then failed. Re-read the row so the list reflects that:
      // an Edit opened against a stale row would show an empty Client ID over
      // the stored one, and saving it would delete what was just supplied.
      if (clientId != null) void refreshRows();
      // The one failure that is really a request for input: show the field
      // instead of only reporting that something is missing.
      if (mcpServerErrorCode(cause) === "client_registration_unavailable") {
        setClientDetailsFor(server);
        return;
      }
      setSignInError({ id: server.id, message });
    } finally {
      setSignInPending(null);
    }
  };

  const signOut = async (server: McpServerView) => {
    if (signInPending != null) return;
    setSignInPending(server.id);
    setSignInError(null);
    setSignOutError(null);
    try {
      const updated = await mcpServerSignOut(server.id);
      replaceRow(updated);
      setPendingSignOut(null);
      toast.success(
        updated.sign_in === "signed_in_for_account"
          ? `Removed this device’s sign-in to “${serverTitle(server)}”. Workspace sign-in remains available.`
          : server.sign_in === "signed_in_for_account"
            ? `Signed out of “${serverTitle(server)}” for this account.`
            : `Signed out of “${serverTitle(server)}” on this device.`,
      );
      onRegistryChanged?.();
    } catch (cause: unknown) {
      const message = describeMcpServerError(cause, "This server could not be signed out of.");
      setSignInError({
        id: server.id,
        message,
      });
      if (pendingSignOut?.id === server.id) setSignOutError(message);
    } finally {
      setSignInPending(null);
    }
  };

  const registered = state.status === "ready" ? state.servers.filter((server) => !server.builtin) : [];
  const registryBusy = signInPending !== null || deleting || restoring;

  return (
    <SettingsGroup id="settings-mcp" aria-labelledby="settings-mcp-heading">
      <SettingsGroupHeader headingId="settings-mcp-heading" title="MCP servers">
        <SettingsButton disabled={state.status === "loading" || registryBusy} icon={<RefreshCw />} onClick={reload}>
          Refresh
        </SettingsButton>
        <SettingsButton disabled={registryBusy} icon={<Plus />} onClick={() => setSheet({ server: null })}>
          Add server
        </SettingsButton>
      </SettingsGroupHeader>

      {state.status === "loading" ? (
        <div className="settings-group__state text-secondary">
          <Spinner decorative size={16} />
          <span>Loading MCP servers…</span>
        </div>
      ) : null}

      {state.status === "error" ? (
        <Alert className="settings-group__state" title="MCP servers unavailable" tone="danger">
          <p>{state.message}</p>
          <SettingsButton className="mt-2" onClick={reload}>
            Try again
          </SettingsButton>
        </Alert>
      ) : null}

      {state.status === "ready" ? (
        <List className="flex flex-col">
          {state.servers.map((server) => (
            <ListItem key={server.id}>
              {server.builtin ? (
                <BundledServerCard
                  server={server}
                  actionsDisabled={registryBusy}
                  onDelete={() => {
                    setDeleteError(null);
                    setPendingDelete(server);
                  }}
                />
              ) : (
                <SearchRevealHost id={settingsItemElementId("mcp", server.id)}>
                  <RegisteredServerCard
                    actionsDisabled={registryBusy}
                    busy={signInPending === server.id}
                    error={signInError?.id === server.id ? signInError.message : null}
                    server={server}
                    onDelete={() => {
                      setDeleteError(null);
                      setPendingDelete(server);
                    }}
                    onEdit={() => setSheet({ server })}
                    onSignIn={() => void signIn(server, null)}
                    onSignOut={() => {
                      if (server.sign_in === "signed_in_for_account") {
                        setSignInError(null);
                        setSignOutError(null);
                        setPendingSignOut(server);
                        return;
                      }
                      void signOut(server);
                    }}
                  />
                </SearchRevealHost>
              )}
            </ListItem>
          ))}
        </List>
      ) : null}

      {state.status === "ready" && !state.servers.some((server) => server.builtin) ? (
        <div className="settings-group__state">
          <SettingsButton disabled={registryBusy} loading={restoring} onClick={() => void restoreBundled()}>
            Restore workspace MCP server
          </SettingsButton>
          {restoreError ? (
            <p className="type-caption text-danger" role="alert">
              {restoreError}
            </p>
          ) : null}
        </div>
      ) : null}

      {state.status === "ready" && registered.length === 0 ? (
        <div className="settings-group__state text-secondary">
          <strong className="font-semibold text-foreground">No additional servers</strong>
          <span>Add one to connect more tools.</span>
        </div>
      ) : null}

      <McpServerSheet
        onClose={() => setSheet(null)}
        onSaved={(saved) => {
          const previous = sheet?.server ?? null;
          setSheet(null);
          if (
            saved.auth_type === "oauth" &&
            (previous == null ||
              // The client this device authenticates as changed, so the grant
              // held under the old one no longer describes this connection.
              previous.oauth_client_id !== saved.oauth_client_id ||
              previous.oauth_client_confidential !== saved.oauth_client_confidential ||
              saved.sign_in === "not_signed_in" ||
              previous.url !== saved.url ||
              previous.auth_type !== "oauth")
          ) {
            // Match xfalcon-web: start OAuth immediately after create so the
            // operator finishes the grant in one motion instead of hunting for
            // a separate sign-in button on the new row.
            setState((current) =>
              current.status === "ready"
                ? {
                    status: "ready",
                    servers: current.servers.some((server) => server.id === saved.id)
                      ? current.servers.map((server) => (server.id === saved.id ? saved : server))
                      : [...current.servers, saved],
                  }
                : current,
            );
            // No client details are passed: saving already sealed them to this
            // device, and sign-in reads them from there. That is what makes a
            // declined or timed-out browser flow recoverable -- the row keeps
            // the client, and "Sign in" retries with it.
            void signIn(saved, null);
          } else {
            reload();
          }
          onRegistryChanged?.();
        }}
        open={sheet != null}
        server={sheet?.server ?? null}
      />

      <AlertDialog
        busy={signInPending === pendingSignOut?.id}
        confirmLabel="Sign out everyone"
        description={pendingSignOut ? describeSignOut(pendingSignOut.sign_in ?? "signed_in") : ""}
        destructive
        error={signOutError}
        open={pendingSignOut != null}
        onCancel={() => {
          setPendingSignOut(null);
          setSignOutError(null);
        }}
        onConfirm={() => {
          if (pendingSignOut) void signOut(pendingSignOut);
        }}
        title={pendingSignOut ? `Sign out of “${serverTitle(pendingSignOut)}”?` : "Sign out?"}
      />

      <McpClientDetailsDialog
        busy={signInPending === clientDetailsFor?.id}
        onCancel={() => setClientDetailsFor(null)}
        onSubmit={(clientId, clientSecret) => {
          if (clientDetailsFor) void signIn(clientDetailsFor, clientId, clientSecret);
        }}
        server={clientDetailsFor ? { id: clientDetailsFor.id, label: serverTitle(clientDetailsFor) } : null}
        error={signInError?.id === clientDetailsFor?.id ? signInError?.message : null}
      />

      <AlertDialog
        busy={deleting}
        confirmLabel="Remove server"
        description={
          pendingDelete?.builtin
            ? "Stops using the workspace MCP server on this device for new chat runs. The xFalcon account stays signed in. You can restore the connection here."
            : "Removes the server from all clients on this account and deletes its credential on this device. This action cannot be undone."
        }
        destructive
        error={deleteError}
        open={pendingDelete != null}
        onCancel={() => {
          setPendingDelete(null);
          setDeleteError(null);
        }}
        onConfirm={() => void confirmDelete()}
        title={pendingDelete ? `Remove “${serverTitle(pendingDelete)}”?` : "Remove server?"}
      />
    </SettingsGroup>
  );
}

function serverTitle(server: McpServerView): string {
  return server.builtin ? "Workspace MCP server" : (server.display_name ?? server.name);
}

/**
 * The packaged endpoint uses the app's browser session. Removing this
 * connection leaves workspace authentication intact.
 */
function BundledServerCard({
  server,
  actionsDisabled,
  onDelete,
}: {
  server: McpServerView;
  actionsDisabled: boolean;
  onDelete: () => void;
}) {
  return (
    <div className="settings-collection-item settings-collection-item--card">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <Server aria-hidden className="text-muted" size={16} strokeWidth={1.8} />
            <strong className="type-body break-words text-foreground">Workspace MCP server</strong>
            <Badge tone="success">Signed in</Badge>
          </div>
          {server.url ? <p className="type-caption break-all font-mono text-muted">{server.url}</p> : null}
          <p className="type-caption text-muted">{`${MCP_TRANSPORT_LABEL[server.transport]} · Browser sign-in`}</p>
        </div>
        <SettingsActions>
          <SettingsRemoveButton
            aria-label={`Remove ${serverTitle(server)}`}
            disabled={actionsDisabled}
            onClick={onDelete}
          />
        </SettingsActions>
      </div>
    </div>
  );
}

/** Which row a sign-in failure belongs to, so it is shown on that row alone. */
interface RowError {
  id: string;
  message: string;
}

interface RegisteredServerCardProps {
  server: McpServerView;
  /** A sign-in or sign-out is in flight for this row. */
  busy: boolean;
  /** The last sign-in failure for this row, already turned into a sentence. */
  error: string | null;
  actionsDisabled: boolean;
  onEdit: () => void;
  onDelete: () => void;
  onSignIn: () => void;
  onSignOut: () => void;
}

function RegisteredServerCard({
  server,
  busy,
  error,
  actionsDisabled,
  onEdit,
  onDelete,
  onSignIn,
  onSignOut,
}: RegisteredServerCardProps) {
  const reason = server.unavailable;
  const unavailableDetail = reason ? MCP_UNAVAILABLE_DETAIL[reason] : null;
  const signIn = server.sign_in;
  const transportHandling = describeTransportHandling(server.transport);
  const [detailsOpen, setDetailsOpen] = React.useState(reason != null);
  const summary = [
    MCP_TRANSPORT_LABEL[server.transport],
    MCP_AUTH_TYPE_LABEL[server.auth_type],
    MCP_PERMISSION_MODE[server.permission_mode].label,
  ].join(" · ");
  // One primary status: a server that is not in use says so, otherwise its
  // sign-in state, otherwise that it is enabled. Disabled is shown alongside.
  let status: React.ReactNode = null;
  if (reason) status = <Badge tone="warn">Not in use</Badge>;
  else if (signIn) status = <Badge tone={MCP_SIGN_IN_STATUS[signIn].tone}>{MCP_SIGN_IN_STATUS[signIn].label}</Badge>;
  else if (server.enabled) status = <Badge tone="success">Enabled</Badge>;

  return (
    <div className="settings-collection-item settings-collection-item--card">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <strong className="type-body break-words text-foreground">
              <HighlightedText text={serverTitle(server)} />
            </strong>
            {status}
            {server.enabled ? null : <Badge tone="neutral">Disabled</Badge>}
          </div>
          {server.description ? <p className="type-caption break-words text-secondary">{server.description}</p> : null}
          <p className="settings-collection-item__summary">{summary}</p>
        </div>
        <SettingsActions>
          {signIn != null && signIn !== "not_signed_in" ? (
            <SettingsButton disabled={actionsDisabled} icon={<LogOut />} loading={busy} onClick={onSignOut}>
              Sign out
            </SettingsButton>
          ) : null}
          <SettingsButton
            aria-label={`Edit ${serverTitle(server)}`}
            disabled={actionsDisabled}
            icon={<Pencil />}
            onClick={onEdit}
          >
            Edit
          </SettingsButton>
          <SettingsRemoveButton
            aria-label={`Remove ${serverTitle(server)}`}
            disabled={actionsDisabled}
            onClick={onDelete}
          />
        </SettingsActions>
      </div>

      {signIn != null && signIn !== "not_signed_in" ? (
        <p className="type-caption text-muted">{MCP_SIGN_IN_STATUS[signIn].detail}</p>
      ) : null}

      <button
        aria-expanded={detailsOpen}
        className="settings-disclosure"
        type="button"
        onClick={() => setDetailsOpen((open) => !open)}
      >
        {detailsOpen ? "Hide details" : "Show details"}
      </button>

      {detailsOpen ? (
        <div className="settings-collection-item__details">
          {server.url ? <p className="type-caption break-all font-mono text-muted">{server.url}</p> : null}
          <dl className="type-caption grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-muted">
            <dt>Transport</dt>
            <dd className="text-secondary">
              {MCP_TRANSPORT_LABEL[server.transport]}
              {transportHandling ? <span className="text-muted"> — {transportHandling}</span> : null}
            </dd>
            <dt>Authentication</dt>
            <dd className="text-secondary">{MCP_AUTH_TYPE_LABEL[server.auth_type]}</dd>
            <dt>Tool approvals</dt>
            <dd className="text-secondary">
              {MCP_PERMISSION_MODE[server.permission_mode].label} —{" "}
              {MCP_PERMISSION_MODE[server.permission_mode].meaning}
            </dd>
          </dl>
        </div>
      ) : null}

      {reason ? (
        <Alert title="Not being used on this device" tone="warn">
          <p>{MCP_UNAVAILABLE_EXPLANATION[reason]}</p>
          {unavailableDetail ? <p className="mt-1">{unavailableDetail}</p> : null}
          {reason === "needs_sign_in" ? (
            <SettingsButton
              className="mt-2"
              disabled={actionsDisabled}
              icon={<LogIn />}
              loading={busy}
              onClick={onSignIn}
            >
              Sign in with this server
            </SettingsButton>
          ) : null}
        </Alert>
      ) : null}

      {busy ? (
        <p className="type-caption flex items-center gap-2 text-secondary">
          <Spinner decorative size={14} />
          Complete sign-in in the browser.
        </p>
      ) : null}

      {error ? (
        <p className="type-caption text-danger" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
