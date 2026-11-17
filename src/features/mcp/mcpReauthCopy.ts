/**
 * Copy for the reconnect prompt (spec: mcp-reauth-prompt).
 *
 * Kept beside the component rather than in `settingsCopy` because this surface
 * is not part of Settings — it is raised over whatever the user was doing when a
 * run hit a refused server.
 *
 * The words name the server and name the remedy. "Authentication error" would be
 * accurate and useless: the whole point is that the user can fix this in one
 * click, and the sentence has to say so.
 */
export const MCP_REAUTH_COPY = {
  title: (label: string) => `Reconnect ${label}`,
  credentialRefused: "The session has expired or requires authorization. Sign in to restore access to this server.",
  insufficientScope:
    "This server's saved sign-in does not cover what it now requires, so its tools cannot run. Signing in again requests the access it asks for.",
  reconnectLabel: "Reconnect",
  dismissLabel: "Dismiss",
  reconnected: (label: string) => `${label} is connected again.`,
  reconnectFailed: "Sign-in did not complete. Try again.",
  wrongServer: "The connection check returned a different server. Try reconnecting again.",
  stillRefused: "The server is not connected. Try again.",
} as const;
