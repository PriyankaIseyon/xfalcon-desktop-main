/**
 * Destinations the palette can jump to inside Settings.
 *
 * Keywords are the words a reader actually types when they are looking for
 * the control, not the heading: "dark" for Appearance, "api key" for
 * Models & connections. The section id is the element SettingsView scrolls to.
 */
export const SETTINGS_DESTINATIONS = [
  {
    id: "appearance",
    rowId: "appearance",
    label: "Appearance",
    keywords: "theme dark light system reading size font type scale",
  },
  {
    id: "models-connections",
    rowId: "models-connections",
    label: "Models & connections",
    keywords: "models connections providers api keys private custom mcp servers",
  },
  {
    id: "models-connections",
    rowId: "models",
    label: "Default model",
    keywords: "models default sonnet grok luna sol new chats openrouter private custom",
  },
  {
    id: "models-connections",
    rowId: "private-models",
    label: "Private models",
    keywords: "private custom openai compatible endpoint key",
  },
  {
    id: "models-connections",
    rowId: "providers",
    label: "Model providers",
    keywords: "providers models api key openai anthropic openrouter credential",
  },
  {
    id: "models-connections",
    rowId: "mcp",
    label: "MCP servers",
    keywords: "mcp tools servers sign in oauth",
  },
  {
    id: "agent-capabilities",
    rowId: "agent-capabilities",
    label: "Agent capabilities",
    keywords: "agent capabilities skills import instructions library reasoning try next suggestions follow ups chips",
  },
  {
    id: "agent-capabilities",
    rowId: "skills",
    label: "Skills",
    keywords: "skills import agent instructions library",
  },
  {
    id: "agent-capabilities",
    rowId: "reasoning",
    label: "Model reasoning",
    keywords: "reasoning thinking panel show model",
  },
  {
    id: "agent-capabilities",
    rowId: "suggestions",
    label: "Try next suggestions",
    keywords: "suggestions follow ups chips try next prompts",
  },
  {
    id: "application",
    rowId: "application",
    label: "Application",
    keywords: "updates version revision build commit restart install release",
  },
  {
    id: "application",
    rowId: "updates",
    label: "Software updates",
    keywords: "updates version check install restart release",
  },
  {
    id: "application",
    rowId: "diagnostics",
    label: "Support",
    keywords: "logs troubleshooting bundle export support diagnostics help email",
  },
  {
    id: "application",
    rowId: "security-report",
    label: "Report a security issue",
    keywords: "security report vulnerability incident urgent phishing breach leak",
  },
  {
    id: "application",
    rowId: "data-reset",
    label: "Data & reset",
    keywords: "reset erase delete wipe factory fresh install privacy purge",
  },
] as const;

export type SettingsSectionId = "appearance" | "models-connections" | "agent-capabilities" | "application";

export function settingsSectionElementId(id: SettingsSectionId): string {
  return `settings-section-${id}`;
}

export function settingsItemElementId(kind: "mcp" | "skill", key: string): string {
  return `settings-${kind}-${key.replace(/[^a-zA-Z0-9_-]+/g, "-")}`;
}

export type SettingsReveal = {
  section: SettingsSectionId;
  targetId: string;
  query: string;
  token: number;
};

export function settingsSectionReveal(section: SettingsSectionId, query = ""): Omit<SettingsReveal, "token"> {
  return { section, targetId: settingsSectionElementId(section), query };
}

type SettingsDestinationRowId = (typeof SETTINGS_DESTINATIONS)[number]["rowId"];

/** Where each palette destination lands: its section, and the element to scroll to inside it when it has one. */
const SETTINGS_DESTINATION_TARGETS: Record<
  SettingsDestinationRowId,
  { section: SettingsSectionId; targetId?: string }
> = {
  appearance: { section: "appearance" },
  "models-connections": { section: "models-connections" },
  models: { section: "models-connections", targetId: "settings-default-model" },
  "private-models": { section: "models-connections", targetId: "settings-private-models" },
  providers: { section: "models-connections", targetId: "settings-providers" },
  mcp: { section: "models-connections", targetId: "settings-mcp" },
  "agent-capabilities": { section: "agent-capabilities" },
  reasoning: { section: "agent-capabilities", targetId: "settings-show-reasoning-row" },
  suggestions: { section: "agent-capabilities", targetId: "settings-show-follow-ups-row" },
  skills: { section: "agent-capabilities", targetId: "settings-skills" },
  application: { section: "application" },
  updates: { section: "application", targetId: "settings-updates" },
  diagnostics: { section: "application", targetId: "settings-support" },
  "security-report": { section: "application", targetId: "settings-security-report" },
  "data-reset": { section: "application", targetId: "settings-reset" },
};

export function settingsDestinationReveal(
  rowId: SettingsDestinationRowId,
  query: string,
): Omit<SettingsReveal, "token"> {
  const { section, targetId } = SETTINGS_DESTINATION_TARGETS[rowId];
  return { section, targetId: targetId ?? settingsSectionElementId(section), query };
}
