import type { ChatErrorCode } from "../../lib/generated/contracts";

/**
 * What a rejected chat command says to the reader.
 *
 * The backend already writes this prose (`ChatServiceError`'s `Display`), and
 * `describeChatError` renders it verbatim. This map is the floor under that: a
 * rejection that arrives without a usable message still becomes a sentence
 * rather than an enum name, and a new arm in `crates/contracts` fails the build
 * here instead of reaching a user untranslated.
 *
 * Chat used to interpolate `cause.message` directly, which is how
 * "chat is unavailable" and "conversation_not_found" reached the surface. No
 * code and no error object is rendered directly any more.
 */
const CHAT_ERROR_MESSAGE: Record<ChatErrorCode, string> = {
  auth_required: "Sign in to use chat.",
  not_configured: "Auth is not configured on this device.",
  session_expired: "Session expired. Sign in again.",
  no_active_account: "No account is active on this device.",
  send_in_flight: "A message is already sending. Wait for it to finish.",
  too_many_runs: "Too many active runs. Cancel one and try again.",
  not_found: "That conversation no longer exists.",
  temporarily_unavailable: "The workspace is not responding right now. Try again in a moment.",
  service_unavailable: "The workspace could not complete that. Reload xFalcon, then try again.",
  failed: "That could not be completed.",
};

/**
 * A Tauri command rejection is the serialized error *value*, not an `Error`, so
 * a typed backend failure arrives as `{ version, code, message }` and must be
 * recognised structurally. Mirrors `settingsCopy`'s `codeOf`.
 */
function typedChatError(cause: unknown): { code: ChatErrorCode; message: string } | null {
  if (typeof cause !== "object" || cause === null) return null;
  const { code, message } = cause as { code?: unknown; message?: unknown };
  if (typeof code !== "string" || !Object.hasOwn(CHAT_ERROR_MESSAGE, code)) return null;
  const prose =
    typeof message === "string" && message.trim() ? message.trim() : CHAT_ERROR_MESSAGE[code as ChatErrorCode];
  return { code: code as ChatErrorCode, message: prose };
}

/**
 * The sentence for a failed chat command.
 *
 * `fallback` covers the one case the contract cannot: the IPC bridge itself did
 * not answer, so there is no typed value to read. That is the only situation in
 * which a caller may claim the secure runtime is at fault -- saying it for a
 * workspace outage is what sent readers to debug the wrong machine.
 */
export function describeChatError(cause: unknown, fallback: string): string {
  return typedChatError(cause)?.message ?? fallback;
}

/** The typed code behind a rejection, for the decisions prose cannot carry. */
export function chatErrorCode(cause: unknown): ChatErrorCode | null {
  return typedChatError(cause)?.code ?? null;
}

/**
 * Whether waiting is the honest advice.
 *
 * Only `temporarily_unavailable` qualifies: the workspace was unreachable, 5xx,
 * timed out, or rate limited, and the same request can succeed unchanged.
 * Surfaces keep their retry affordance alive on this and retire it otherwise,
 * so a button that cannot win is not offered.
 */
export function chatErrorIsTransient(cause: unknown): boolean {
  return chatErrorCode(cause) === "temporarily_unavailable";
}
