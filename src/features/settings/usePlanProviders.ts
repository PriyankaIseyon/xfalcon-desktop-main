import { useCallback, useEffect, useRef, useState } from "react";
import type { ChatgptPlanStatusView, ClaudeEffort, ClaudeRuntimeStatusView } from "../../lib/generated/contracts";
import {
  chatgptPlanSignIn,
  chatgptPlanSignInCancel,
  chatgptPlanSignOut,
  chatgptPlanStatus,
  chatgptPlanWelcomeAck,
  claudeRuntimeSetEffort,
  claudeRuntimeSetEnabled,
  claudeRuntimeStatus,
} from "../../lib/native";
import { CHATGPT_PLAN_COPY, CLAUDE_RUNTIME_COPY } from "./settingsCopy";

function useMountedRef() {
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  return mounted;
}

/**
 * Runs one backend command for a hook. `busy` counts the commands still in
 * flight, so it clears only when the last one settles. The promise resolves to
 * the command's value, or to `null` after it sets `error`.
 *
 * A command that settles after unmount still resolves, so its caller can report
 * a change that really happened. It writes no state, because nothing is left to
 * show it.
 */
function useCommandRunner() {
  const mounted = useMountedRef();
  const [inFlight, setInFlight] = useState(0);
  const [error, setError] = useState<string | null>(null);

  const run = useCallback(
    async <T>(command: () => Promise<T>, failure: string): Promise<T | null> => {
      setInFlight((count) => count + 1);
      setError(null);
      try {
        return await command();
      } catch {
        if (mounted.current) setError(failure);
        return null;
      } finally {
        if (mounted.current) setInFlight((count) => count - 1);
      }
    },
    [mounted],
  );

  return { busy: inFlight > 0, error, setError, run, mounted };
}

/** The Claude runtime on this device. The status loads on mount. */
export function useClaudeRuntime() {
  const { busy, error, run, mounted } = useCommandRunner();
  const [status, setStatus] = useState<ClaudeRuntimeStatusView | null>(null);

  const refresh = useCallback(async () => {
    const next = await run(claudeRuntimeStatus, CLAUDE_RUNTIME_COPY.loadFailed);
    if (next && mounted.current) setStatus(next);
    return next !== null;
  }, [mounted, run]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const setEnabled = useCallback(
    async (enabled: boolean) => {
      const next = await run(() => claudeRuntimeSetEnabled({ enabled }), CLAUDE_RUNTIME_COPY.setFailed);
      if (next && mounted.current) setStatus(next);
      return next !== null;
    },
    [mounted, run],
  );

  const setEffort = useCallback(
    async (effort: ClaudeEffort) => {
      const next = await run(() => claudeRuntimeSetEffort({ effort }), CLAUDE_RUNTIME_COPY.setFailed);
      if (next && mounted.current) setStatus(next);
      return next !== null;
    },
    [mounted, run],
  );

  return { status, busy, error, refresh, setEnabled, setEffort };
}

/** The ChatGPT plan on this device. The status loads on mount. */
export function useChatgptPlan() {
  const { busy, error, setError, run, mounted } = useCommandRunner();
  const [status, setStatus] = useState<ChatgptPlanStatusView | null>(null);
  const [signingIn, setSigningIn] = useState(false);
  // Set when the user cancels, so a sign-in that ends because of it is not reported as a failure.
  const cancelRequested = useRef(false);
  // Whether this hook has a sign-in in flight. Without one, a cancel has nothing that will report back.
  const signInPending = useRef(false);

  // Resolves to the status the command settled with, or null after it sets `error`.
  const runStatus = useCallback(
    async (command: () => Promise<ChatgptPlanStatusView>, failure: string) => {
      const next = await run(command, failure);
      if (next && mounted.current) setStatus(next);
      return next;
    },
    [mounted, run],
  );

  const refresh = useCallback(async () => {
    const next = await runStatus(chatgptPlanStatus, CHATGPT_PLAN_COPY.checkFailed);
    return next !== null;
  }, [runStatus]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const signIn = useCallback(async () => {
    cancelRequested.current = false;
    signInPending.current = true;
    setSigningIn(true);
    const next = await runStatus(chatgptPlanSignIn, CHATGPT_PLAN_COPY.signInFailed);
    signInPending.current = false;
    if (next === null && cancelRequested.current) {
      // The sign-in ended because of the cancel: read the status instead of reporting a failure.
      if (mounted.current) setError(null);
      await refresh();
    }
    if (mounted.current) setSigningIn(false);
    // A failed re-sign-in stays `ready` with its reason in `error`, and a cancel
    // resolves signed out. Neither changed the account.
    return next !== null && next.state === "ready" && !next.error;
  }, [mounted, refresh, runStatus, setError]);

  const cancelSignIn = useCallback(async () => {
    cancelRequested.current = true;
    const cancelled = await run(chatgptPlanSignInCancel, CHATGPT_PLAN_COPY.cancelFailed);
    if (cancelled !== null && !signInPending.current) await refresh();
  }, [refresh, run]);

  // Sign-out clears the local record even when OpenAI does not confirm the
  // revocation, so the state alone decides here, not `error`.
  const signOut = useCallback(async () => {
    const next = await runStatus(chatgptPlanSignOut, CHATGPT_PLAN_COPY.signOutFailed);
    return next !== null && next.state === "signed_out";
  }, [runStatus]);

  const acknowledgeWelcome = useCallback(async () => {
    const next = await runStatus(chatgptPlanWelcomeAck, CHATGPT_PLAN_COPY.welcomeFailed);
    return next !== null && !next.error;
  }, [runStatus]);

  return { status, busy, signingIn, error, refresh, signIn, cancelSignIn, signOut, acknowledgeWelcome };
}
