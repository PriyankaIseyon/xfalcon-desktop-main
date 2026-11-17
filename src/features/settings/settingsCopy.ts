import type { ClaudeEffort, CredentialErrorCode } from "../../lib/generated/contracts";
import type {
  CredentialStatus,
  McpAuthType,
  McpConnectionState,
  McpPermissionMode,
  McpServerErrorCode,
  McpServerUnavailableReason,
  McpSignInStatus,
  McpTransport,
  ProviderId,
  ReadingSize,
  SkillErrorCode,
  ThemePreference,
} from "../../lib/generated/contracts";
import type { BadgeTone } from "../../ui/Badge";

// Every map here is a total `Record` over a contract union on purpose: a new arm
// in `crates/contracts` then fails the build instead of reaching a user as its
// own enum name. The backend hands the frontend redacted codes and nothing else,
// so this module is where those codes become sentences -- no code, and no error
// object, is ever rendered directly.

/** The primary, plain-language reason a listed server is not being used. */
export const MCP_UNAVAILABLE_EXPLANATION: Record<McpServerUnavailableReason, string> = {
  unsupported_transport: "This server requires a remote HTTPS endpoint.",
  unsupported_auth_method: "This authentication method is not supported.",
  needs_sign_in: "Sign in to this server on this device to use it.",
  missing_device_credential: "Add this server's credential on this device to use it.",
  invalid_url: "This server's address cannot be used from the desktop app.",
};

/**
 * A second sentence, where the first could be misread as "this server is
 * broken". A server definition is shared across the account; a credential is
 * kept per device, so a missing one says nothing about the server itself.
 */
export const MCP_UNAVAILABLE_DETAIL: Record<McpServerUnavailableReason, string | null> = {
  unsupported_transport: "Local process connections are not supported. Configure a remote HTTPS endpoint.",
  unsupported_auth_method: "Supported methods are OAuth, bearer tokens, and custom headers.",
  needs_sign_in: "Browser sign-in is required on each device. Credentials remain on that device.",
  missing_device_credential:
    "The server is shared across the account. Credentials must be added separately on each device.",
  invalid_url: "A server must be reachable at an https address on a public host.",
};

/**
 * What a signed-in or signed-out `oauth` row says about itself.
 *
 * `signed_in_for_account` is the state a workspace-wide grant is in — the same one
 * xFalcon on the web uses. It reads differently on purpose: signing out of it
 * signs out every client, which the confirmation below spells out.
 */
export const MCP_SIGN_IN_STATUS: Record<McpSignInStatus, { label: string; tone: BadgeTone; detail: string }> = {
  signed_in: {
    label: "Signed in on this device",
    tone: "success",
    detail: "Applies to this device only.",
  },
  signed_in_for_account: {
    label: "Signed in for the workspace",
    tone: "success",
    detail: "Shared across xFalcon clients in the workspace.",
  },
  not_signed_in: {
    label: "Not signed in on this device",
    tone: "warn",
    detail: "Sign-in opens the default browser.",
  },
};

/** What signing out actually costs, which differs by how far the sign-in reaches. */
export function describeSignOut(status: McpSignInStatus): string {
  return status === "signed_in_for_account"
    ? "Signs out all clients on this account, including xFalcon Web. A new sign-in is required to use this server."
    : "Signs out this device only. Other devices remain signed in.";
}

export const MCP_CONNECTION_STATE: Record<McpConnectionState, { label: string; tone: BadgeTone }> = {
  connected: { label: "Connected", tone: "success" },
  rejected: { label: "Connection rejected", tone: "danger" },
  unreachable: { label: "Unreachable", tone: "danger" },
  not_attempted: { label: "Not attempted", tone: "warn" },
};

/** Plan D6: what each mode actually does to an approval pause. */
export const MCP_PERMISSION_MODE: Record<McpPermissionMode, { label: string; meaning: string }> = {
  allow: {
    label: "Allow",
    meaning: "Read-only tools run without approval. Tools that can change data require approval.",
  },
  ask: { label: "Ask", meaning: "Every tool call requires approval." },
  deny: {
    label: "Deny",
    meaning: "All tools from this server are disabled.",
  },
};

/**
 * What this app does with a row whose declared transport is not the one it
 * speaks, for the rows it dials anyway.
 *
 * A row labelled `sse` is dialled with Streamable HTTP: the clients that write
 * these rows do not distinguish the two — xFalcon Web labels every server it
 * registers `sse` — so the label is not treated as authority on the wire
 * protocol. Saying so on the row keeps the displayed label from reading as a
 * contradiction.
 */
export function describeTransportHandling(transport: McpTransport): string | null {
  return transport === "sse" ? "Desktop uses Streamable HTTP." : null;
}

export const MCP_TRANSPORT_LABEL: Record<McpTransport, string> = {
  http: "Streamable HTTP",
  sse: "Server-sent events",
  stdio: "Local process",
  other: "Unrecognized transport",
};

export const MCP_AUTH_TYPE_LABEL: Record<McpAuthType, string> = {
  none: "No authentication",
  header: "Custom header",
  bearer: "Bearer token",
  oauth: "OAuth",
  other: "Unrecognized method",
};

/**
 * `MCP_AUTH_TYPE_LABEL` covers every method the contract can *describe*, because
 * a row registered by another client still has to be named on its card. It is
 * not a menu: this app registers `oauth` alone -- see `McpServerSheet`.
 */

const MCP_SERVER_ERROR_MESSAGE: Record<McpServerErrorCode, string> = {
  invalid_request: "Invalid server details. Check the name, address, and header name.",
  url_rejected: "A public HTTPS address is required.",
  conflict: "Another server on this account already uses that name. Choose a different one.",
  not_editable: "This server is provided by the workspace and cannot be changed here.",
  not_found: "This server is no longer registered on this account. Refresh the list.",
  unauthorized: "The session is not authorized. Sign in again.",
  storage_unavailable: "This device could not store the credential, so nothing was changed. Try again.",
  service_unavailable: "The server registry could not be reached. Try again in a moment.",
  discovery_failed:
    "OAuth configuration is unavailable or unsupported. Check the address or contact the server administrator.",
  // Not a failure so much as a request for the one thing that cannot be
  // discovered. `McpServersSection` shows the field rather than only this line.
  client_registration_unavailable: "Enter the client ID supplied by the server administrator, then sign in again.",
  sign_in_declined: "Sign-in was not completed in the browser. Nothing was changed.",
  sign_in_timed_out: "Sign-in timed out. Try again.",
  sign_in_failed: "This server refused the sign-in. Check the client details, then try again.",
};

const CREDENTIAL_ERROR_MESSAGE: Record<CredentialErrorCode, string> = {
  invalid_request: "Check the API key and try again.",
  invalid_credential: "The provider rejected the key. Check the key and re-enter it.",
  insufficient_permissions: "Permission to add provider keys is required. Contact a workspace administrator.",
  provider_unavailable: "The provider could not be reached to verify that key. Try again in a moment.",
  rate_limited: "A provider or xFalcon rate limit was reached. Wait a moment, then try again.",
  storage_unavailable: "This device could not open a legacy stored key. Re-enter it to continue.",
  service_unavailable: "Provider keys could not be saved or retrieved. Try again.",
  conflict: "The key list has changed. Refresh the list and try again.",
  reconciliation_required: "Key save status is unknown. Refresh the list before retrying.",
  unauthorized: "The session is not authorized. Sign in again.",
};

/**
 * A Tauri command rejection is the serialized error *value*, not an `Error`, so
 * a typed backend failure arrives as `{ version, code }` and must be recognised
 * structurally. Anything unrecognisable falls back to the caller's sentence
 * rather than being stringified at the user.
 */
function codeOf<T extends string>(cause: unknown, messages: Record<T, string>): T | null {
  if (typeof cause !== "object" || cause === null) return null;
  const code = (cause as { code?: unknown }).code;
  return typeof code === "string" && Object.hasOwn(messages, code) ? (code as T) : null;
}

export function describeMcpServerError(cause: unknown, fallback: string): string {
  const code = codeOf<McpServerErrorCode>(cause, MCP_SERVER_ERROR_MESSAGE);
  return code === null ? fallback : MCP_SERVER_ERROR_MESSAGE[code];
}

/**
 * The typed code behind a rejection, for the one case where the *message* is not
 * the whole response: `client_registration_unavailable` means the surface should
 * show the client-ID field, not just say that one is needed.
 */
export function mcpServerErrorCode(cause: unknown): McpServerErrorCode | null {
  return codeOf<McpServerErrorCode>(cause, MCP_SERVER_ERROR_MESSAGE);
}

export function describeCredentialError(cause: unknown, fallback: string): string {
  const code = codeOf<CredentialErrorCode>(cause, CREDENTIAL_ERROR_MESSAGE);
  return code === null ? fallback : CREDENTIAL_ERROR_MESSAGE[code];
}

export const PROVIDER_LABEL: Record<ProviderId, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  google: "Google",
  openrouter: "OpenRouter",
  custom_openai: "Custom model",
  claude_code: "Claude account",
  chatgpt_plan: "ChatGPT plan",
};

/** What a key for each provider looks like, so a key pasted into the wrong row is obvious. */
export const PROVIDER_KEY_HINT: Record<ProviderId, string> = {
  openai: "Starts with sk-",
  anthropic: "Starts with sk-ant-",
  google: "A Google AI Studio API key",
  openrouter: "Starts with sk-or-",
  custom_openai: "Managed with the private model in Settings → Models",
  claude_code: "Uses your signed-in Claude account, not a key",
  chatgpt_plan: "Uses your ChatGPT plan, not a key",
};

export const PROVIDER_IDS: readonly ProviderId[] = ["openai", "anthropic", "google", "openrouter"];

/**
 * The model dropdown's group headings, in the order they show. Each model source
 * names one of these, so a heading cannot be spelt two ways.
 */
export const MODEL_SOURCE_ORDER = ["OpenRouter", "Private models", "Claude account", "ChatGPT plan"] as const;

type ModelSourceGroup = (typeof MODEL_SOURCE_ORDER)[number];
type ModelSource = { badge: string; spoken: string; group: ModelSourceGroup };

const OPENROUTER_MODEL_SOURCE: ModelSource = { badge: "OpenRouter", spoken: "OpenRouter model", group: "OpenRouter" };

/**
 * Where a model runs, as the badge, the screen-reader wording and the dropdown
 * heading it sits under. A total `Record` like the tables above, so a new
 * provider has to choose its wording.
 */
export const MODEL_SOURCE_LABEL: Record<ProviderId, ModelSource> = {
  openai: OPENROUTER_MODEL_SOURCE,
  anthropic: OPENROUTER_MODEL_SOURCE,
  google: OPENROUTER_MODEL_SOURCE,
  openrouter: OPENROUTER_MODEL_SOURCE,
  custom_openai: { badge: "Private", spoken: "private model", group: "Private models" },
  claude_code: { badge: "Claude account", spoken: "Claude account model", group: "Claude account" },
  chatgpt_plan: { badge: "ChatGPT plan", spoken: "ChatGPT plan model", group: "ChatGPT plan" },
};

export const CREDENTIAL_STATUS: Record<CredentialStatus, { label: string; tone: BadgeTone; detail: string | null }> = {
  ready: { label: "Ready", tone: "success", detail: null },
  disabled: { label: "Disabled", tone: "neutral", detail: "This key is registered but will not be used." },
  needs_reentry: {
    label: "Needs re-entry",
    tone: "warn",
    detail: "Re-enter this legacy key to make it available across xFalcon clients.",
  },
  verification_failed: {
    label: "Verification failed",
    tone: "danger",
    detail: "The provider rejected this key the last time it was used.",
  },
};

/**
 * The statuses a fresh paste of the same key fixes, and therefore the rows that
 * offer "Re-enter key".
 *
 * `ready` and `disabled` are excluded because neither is about the secret:
 * a disabled key is a deliberate state, not a broken one.
 */
export const CREDENTIAL_NEEDS_REENTRY: ReadonlySet<CredentialStatus> = new Set<CredentialStatus>([
  "needs_reentry",
  "verification_failed",
]);

// --- Appearance (spec: settings-appearance) --------------------------------

export const APPEARANCE_COPY = {
  title: "Appearance",
  themeLabel: "Theme",
  themeHint: "App color scheme, independent of system settings.",
  readingSizeLabel: "Reading size",
  readingSizeHint: "Text size throughout the app.",
  showReasoningLabel: "Model reasoning",
  showReasoningHint: "Show available model reasoning in a collapsible panel. Does not change model behavior.",
  showFollowUpsLabel: "Try next suggestions",
  showFollowUpsHint: "Show suggested follow-up prompts after assistant replies.",
  saveFailed: "Display preferences applied but could not be saved.",
} as const;

export const THEME_OPTIONS: { value: ThemePreference; label: string }[] = [
  { value: "dark", label: "Dark" },
  { value: "light", label: "Light" },
];

export const READING_SIZE_OPTIONS: { value: ReadingSize; label: string }[] = [
  { value: "small", label: "Small" },
  { value: "medium", label: "Medium" },
  { value: "large", label: "Large" },
];

// --- Skills (spec: user-skills) --------------------------------------------

export const SKILLS_COPY = {
  title: "Skills",
  description:
    "Your enabled skills are available to the agent. They belong to your account only. Full instructions load when needed.",
  importLabel: "Import skills",
  emptyTitle: "No skills imported",
  emptyBody: "Import skill files, archives, or folders.",
  enabledHint: "Available for agent tasks.",
  disabledHint: "Stored but disabled.",
  bundledFiles: (count: number) => (count === 1 ? "1 bundled file" : `${count} bundled files`),
  listFailed: "Imported skills could not be loaded.",
  importFailed: "The selected files could not be imported.",
  toggleFailed: "The skill setting could not be saved.",
  deleteFailed: "The skill could not be removed.",
} as const;

/**
 * The import dialog's copy.
 *
 * It carries the explanation because this is the only place the rules are
 * visible: what counts as a skill file, that a folder is searched rather than
 * read one file at a time, and that a name is an identity so a re-import
 * replaces. Left to a button label, none of that fits.
 */
export const SKILL_IMPORT_COPY = {
  title: "Import skills",
  howItWorks:
    "A skill is a SKILL.md file with a name and description in its header. The files it bundles, such as references, scripts, and assets, are kept with it.",
  acceptsTitle: "Supported formats",
  accepts: [
    ".skill packages and skill folders, with every bundled file.",
    "Single .md skill files.",
    "ZIP archives, including nested skill folders and .skill packages.",
    "Folders, including subfolders.",
  ],
  dropTitle: "Drop skills here",
  dropHint: "Files, ZIP archives, and folders.",
  dropActive: "Release to import",
  orLabel: "or choose",
  chooseFilesLabel: "Files or archive",
  chooseFolderLabel: "Folder",
  chooserNote: "Select files or a folder, or drag both into the import area.",
  replaceNote: "Importing an existing skill name replaces its files and preserves its enabled setting.",
  failuresTitle: "Not imported",
  foundNote: "Each selected file receives an import result. Unrelated files in folders are skipped.",
  importingLabel: "Importing…",
  doneLabel: "Done",
} as const;

const SKILL_ERROR_MESSAGE: Record<SkillErrorCode, string> = {
  not_found: "The skill is no longer available on this device.",
  storage_unavailable: "The skills folder on this device could not be read or written.",
  no_active_account: "Sign in to manage your skills. Each account has its own skills on this device.",
  invalid_choice: "That choice expired. Choose it again.",
  selection_too_large: "That selection has too many files. Choose fewer, or a folder.",
};

export function describeSkillError(cause: unknown, fallback: string): string {
  const code = codeOf<SkillErrorCode>(cause, SKILL_ERROR_MESSAGE);
  return code === null ? fallback : SKILL_ERROR_MESSAGE[code];
}

export function credentialErrorCode(cause: unknown): CredentialErrorCode | null {
  return codeOf<CredentialErrorCode>(cause, CREDENTIAL_ERROR_MESSAGE);
}

// --- Plan providers (spec: desktop-claude-runtime, desktop-chatgpt-plan) ----
// Claude is not affiliated with Anthropic, and no Claude or Anthropic logo is
// used. "Claude Code" appears only in body text, never in a title or a label.

// Total over the flag's levels, so a new level fails the build here instead of
// reaching the select without a label. The order here is the order the select lists them.
const CLAUDE_EFFORT_LEVELS: Record<ClaudeEffort, { label: string; description: string }> = {
  low: { label: "Low", description: "Quick exchanges that you review closely." },
  medium: {
    label: "Medium",
    description: "Day-to-day work with a clear scope; it is Claude's own default for these models.",
  },
  high: {
    label: "High (recommended)",
    description:
      "For work where verification matters: it checks its work and edge cases before answering, and it is the default here.",
  },
  xhigh: { label: "Extra high (xhigh)", description: "Deeper reasoning for harder work, at higher token spend." },
  max: {
    label: "Max",
    description: "The deepest and slowest level, for hard problems only, since it can overthink.",
  },
};

export const CLAUDE_RUNTIME_COPY = {
  title: "Claude (your Claude account)",
  description:
    "Runs the Claude Code program installed on this computer, under your own Claude plan. xFalcon is not affiliated with Anthropic.",
  switchLabel: "Use Claude on this device",
  consentTitle: "Use Claude on this device?",
  consentDescription: "The conversation and tool results are sent to Anthropic under your own plan.",
  consentPoints: [
    "Pro and Max content falls under Anthropic's Consumer Terms.",
    "Team, Enterprise and API use falls under Anthropic's Commercial Terms.",
    "xFalcon never sees or stores your Claude sign-in.",
    "Anthropic may limit or suspend accounts, including for use in third-party apps.",
    "xFalcon is not affiliated with Anthropic.",
  ],
  enable: "Enable",
  cancel: "Cancel",
  effortLabel: "Reasoning effort",
  effortNote:
    "New chats start at this level. A chat keeps the level it started with. Higher levels think longer and use more of your Claude plan.",
  effortLevels: CLAUDE_EFFORT_LEVELS,
  checkAgain: "Check again",
  copyCommand: "Copy",
  commandCopied: "Copied the sign-in command.",
  commandCopyFailed: "The sign-in command could not be copied.",
  loading: "Checking Claude Code…",
  notInstalled: "Claude Code was not found on this computer. Install it, then choose Check again.",
  outdated: (installed: string | null, required: string) =>
    `Claude Code ${installed ?? "on this computer"} is older than the supported ${required}. Update it, then choose Check again.`,
  signedOut: "Not signed in. Run this command in a terminal, then choose Check again:",
  ready: "Ready.",
  plan: (plan: string) => `Plan: ${plan}.`,
  signInMethod: (method: string) => `Sign-in method: ${method}.`,
  usingAccount: "Using Claude account",
  checkFailed: "Could not check Claude Code. Try again.",
  loadFailed: "Could not check Claude Code. Try again.",
  setFailed: "The Claude setting could not be saved. Try again.",
} as const;

export const CHATGPT_PLAN_COPY = {
  title: "Use your ChatGPT plan",
  description:
    "Plan usage is separate from xFalcon's own charges. A Plus plan's five-hour limit is shared by every app that uses the plan.",
  loading: "Checking your ChatGPT plan…",
  signedOut: "Sign in with your ChatGPT account to run chats on your plan.",
  signIn: "Continue with ChatGPT",
  planUsageDisabled: "You signed in, but plan usage is off. Sign in again and allow plan usage.",
  needsSignIn: "Your ChatGPT sign-in expired. Sign in again.",
  signingIn: "Waiting for you to finish in your browser…",
  cancelSignIn: "Cancel",
  signedInAs: (label: string | null) => (label ? `Signed in as ${label}` : "Signed in"),
  usingPlan: "Using ChatGPT plan",
  modelCount: (count: number) => (count === 1 ? "1 model" : `${count} models`),
  manageUsage: "Manage usage",
  signOut: "Sign out",
  checkFailed: "Could not check your ChatGPT plan. Try again.",
  tryAgain: "Try again",
  // For `subscription_sharing_usage_limit_exceeded` on a run. No status state
  // carries that failure, so the run's error surface is where this heading shows.
  usageLimitTitle: "Usage limit reached",
  welcomeTitle: "You're using your ChatGPT plan",
  welcomeBody: "Chats that use a ChatGPT model run on your ChatGPT plan, separate from xFalcon's own charges.",
  welcomeAction: "Got it",
  signInFailed: "Sign-in did not finish. Try again.",
  cancelFailed: "Sign-in could not be cancelled. Try again.",
  signOutFailed: "Could not sign out. Try again.",
  welcomeFailed: "Could not save that choice. Try again.",
} as const;
