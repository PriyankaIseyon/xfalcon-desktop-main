import type { AuthLoginErrorCode } from "../../lib/generated/contracts";

/**
 * What a refused sign-in says on screen.
 *
 * The copy lives here because the backend sends a code, never a sentence: its
 * own messages name the tenant's issuer, the redirect URI the flow built, and
 * the loopback port it tried, and those belong in this process's log rather
 * than in the webview (`src-tauri/src/auth/commands.rs`). One total mapping
 * means a new code cannot compile without someone writing what it says.
 *
 * Each line says what happened and who can act on it — the person, or the
 * administrator who holds the tenant's configuration. "Contact your
 * administrator" for a port that is busy on this device sends someone to the
 * wrong place.
 */
export function describeLoginError(code: AuthLoginErrorCode): string {
  switch (code) {
    case "already_authenticating":
      return "A sign-in is already open in the browser. Finish it, or cancel and start again.";
    case "not_configured":
      return "This build has no workspace configured. Contact the administrator.";
    case "browser_unavailable":
      return "The default browser could not be opened. Set one, then try again.";
    case "discovery_failed":
      return "The workspace could not be reached. Check the connection, then try again.";
    case "issuer_mismatch":
      return "This build is set up for a different sign-in service than the workspace uses. Contact the administrator.";
    case "discovery_metadata_invalid":
      return "The workspace returned sign-in settings this version cannot use. Contact the administrator.";
    case "loopback_unavailable":
      return "The port sign-in needs is in use on this device. Close other xFalcon windows, then try again.";
    case "callback_invalid":
      return "The browser came back with a sign-in this app could not verify. Try again.";
    case "callback_timed_out":
      return "Sign-in was not finished in the browser. Try again.";
    case "redirect_not_registered":
      return "The sign-in service does not accept this app's return address. Contact the administrator.";
    case "token_exchange_failed":
      return "Sign-in could not be completed. Try again.";
    case "identity_failed":
      return "You signed in, but the workspace account could not be confirmed. Contact the administrator.";
    case "device_clock_skew":
      return "This device's clock is more than a minute away from the workspace's, so it reads a new sign-in as not valid yet. Correct the date and time on this device, then try again.";
    case "cancelled":
      return "Sign-in was cancelled.";
    case "account_bind_failed":
      return "You signed in, but this device could not open its local storage for the account. Contact the administrator.";
    default:
      return unreachable(code);
  }
}

/**
 * A code with no copy is a compile error, not a blank message.
 *
 * A build that somehow reaches this — a projection from a newer backend —
 * still says something true rather than nothing.
 */
function unreachable(code: never): string {
  void code;
  return "Sign-in did not complete. Try again.";
}

/** A rejected `auth_login`, read from whatever the boundary rejected with. */
export function loginErrorOf(cause: unknown): AuthLoginErrorCode | null {
  if (typeof cause === "object" && cause !== null && "code" in cause) {
    const record = cause as { code: unknown };
    if (typeof record.code === "string") return record.code as AuthLoginErrorCode;
  }
  return null;
}
