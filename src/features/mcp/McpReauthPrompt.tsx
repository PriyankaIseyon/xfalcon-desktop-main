import React from "react";
import type { McpReauthRequired } from "../../lib/generated/contracts";
import { mcpServerSignIn, mcpServerTest, onMcpReauthRequired } from "../../lib/native";
import { AlertDialog } from "../../ui/AlertDialog";
import { toast } from "../../ui/toastStore";
import { McpClientDetailsDialog } from "./McpClientDetailsDialog";
import { mcpServerErrorCode } from "../settings/settingsCopy";
import { MCP_REAUTH_COPY } from "./mcpReauthCopy";

type Phase = { status: "idle" } | { status: "reconnecting" } | { status: "failed"; message: string };

/**
 * Asks for a fresh sign-in when an MCP server refuses the one it has
 * (spec: mcp-reauth-prompt).
 *
 * Mounted app-wide rather than inside a view, because the refusal happens
 * during a run and the user should not have to be looking at Settings to be
 * told. It renders nothing until the backend says a server was refused.
 *
 * # Why this is modal, and why the scrim does not dismiss
 *
 * A refused credential blocks *every* tool call on that server, so the run
 * silently loses a whole capability — the model then explains it cannot do the
 * thing rather than saying a sign-in expired. A toast is too easy to miss for a
 * fault that severe, and a scrim that dismisses turns a stray click into
 * silently accepting the degraded state. Only Reconnect, Dismiss, or Escape
 * close it — each an explicit choice.
 *
 * Dismissing is not resolving: the server stays refused in Settings, where its
 * own message already says to sign in again.
 */
export function McpReauthPrompt({ onReconnected }: { onReconnected?: () => void }) {
  const [prompts, setPrompts] = React.useState<McpReauthRequired[]>([]);
  const prompt = prompts[0] ?? null;
  const [phase, setPhase] = React.useState<Phase>({ status: "idle" });
  const [needsClientDetails, setNeedsClientDetails] = React.useState(false);
  const reconnectingRef = React.useRef(false);
  const mountedRef = React.useRef(false);

  React.useEffect(() => {
    mountedRef.current = true;
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void onMcpReauthRequired((next) => {
      if (cancelled) return;
      // Keep the active server stable and retain other servers for the next prompt.
      setPrompts((current) => (current.some((item) => item.id === next.id) ? current : [...current, next]));
    })
      .then((stop) => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => {
        if (!cancelled) toast.error("Connection alerts are unavailable. Check server sign-in in Settings.");
      });
    return () => {
      cancelled = true;
      mountedRef.current = false;
      unlisten?.();
    };
  }, []);

  const reconnecting = phase.status === "reconnecting";

  const close = React.useCallback(() => {
    reconnectingRef.current = false;
    setPrompts((current) => current.filter((item) => item.id !== prompt?.id));
    setPhase({ status: "idle" });
    setNeedsClientDetails(false);
  }, [prompt?.id]);

  if (prompt === null) return null;

  const reconnect = async (clientId: string | null = null, clientSecret: string | null = null) => {
    if (reconnectingRef.current) return;
    reconnectingRef.current = true;
    setPhase({ status: "reconnecting" });
    try {
      // Try automatic registration first; request operator details only when
      // the server requires them, using the same flow as Settings.
      await mcpServerSignIn(prompt.id, clientId, clientSecret);
      if (!mountedRef.current) return;
      // Verified, not assumed: a sign-in that completed can still leave a
      // server unreachable, and closing on an unverified success would report
      // a fix that did not happen.
      const report = await mcpServerTest(prompt.id);
      if (!mountedRef.current) return;
      if (report.id !== prompt.id) {
        setPhase({ status: "failed", message: MCP_REAUTH_COPY.wrongServer });
        return;
      }
      if (report.state === "connected") {
        toast.success(MCP_REAUTH_COPY.reconnected(prompt.label));
        close();
        onReconnected?.();
        return;
      }
      setPhase({
        status: "failed",
        message: report.message ?? MCP_REAUTH_COPY.stillRefused,
      });
    } catch (cause: unknown) {
      if (!mountedRef.current) return;
      if (mcpServerErrorCode(cause) === "client_registration_unavailable") {
        setNeedsClientDetails(true);
        setPhase({ status: "idle" });
        return;
      }
      // Cancelled or failed. The prompt stays, because the server is still
      // refused and saying otherwise would be a lie the user acts on.
      setPhase({ status: "failed", message: MCP_REAUTH_COPY.reconnectFailed });
    } finally {
      reconnectingRef.current = false;
    }
  };

  const description =
    prompt.reason === "insufficient_scope" ? MCP_REAUTH_COPY.insufficientScope : MCP_REAUTH_COPY.credentialRefused;

  if (needsClientDetails) {
    return (
      <McpClientDetailsDialog
        server={prompt}
        busy={reconnecting}
        error={phase.status === "failed" ? phase.message : null}
        onCancel={close}
        onSubmit={(clientId, clientSecret) => void reconnect(clientId, clientSecret)}
      />
    );
  }

  return (
    <AlertDialog
      busy={reconnecting}
      cancelLabel={MCP_REAUTH_COPY.dismissLabel}
      confirmLabel={MCP_REAUTH_COPY.reconnectLabel}
      description={description}
      // A stray click on the scrim must not read as accepting the fault.
      dismissOnScrim={false}
      error={phase.status === "failed" ? phase.message : null}
      onCancel={close}
      onConfirm={() => void reconnect()}
      open
      title={MCP_REAUTH_COPY.title(prompt.label)}
    />
  );
}
