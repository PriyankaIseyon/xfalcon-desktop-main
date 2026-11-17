import { Bot, FolderTree, HelpCircle, Plug, Sparkles, Trash2 } from "lucide-react";

export type PaletteKind = "mention" | "slash";

export interface PaletteItem {
  id: string;
  kind: PaletteKind;
  category: "file" | "mcp" | "report" | "command" | "skill";
  title: string;
  subtitle?: string;
  insertText: string;
  icon: React.ReactNode;
  onSelect?: () => void;
}

export type SlashCommandKey = "model" | "clear" | "skill" | "project" | "mcp" | "help";

export const SLASH_COMMAND_KEYS: readonly SlashCommandKey[] = [
  "model",
  "clear",
  "skill",
  "project",
  "mcp",
  "help",
] as const;

export function extractSlashCommand(input: string): SlashCommandKey | null {
  const trimmed = input.trim();
  if (!trimmed.startsWith("/")) return null;
  const withoutSlash = trimmed.slice(1);
  const spaceIndex = withoutSlash.search(/\s/);
  const commandName = spaceIndex === -1 ? withoutSlash.toLowerCase() : withoutSlash.slice(0, spaceIndex).toLowerCase();
  const args = spaceIndex === -1 ? "" : withoutSlash.slice(spaceIndex).trim();

  if (SLASH_COMMAND_KEYS.includes(commandName as SlashCommandKey)) {
    // All current slash commands (/model, /clear, /skill, /project, /mcp, /help)
    // are zero-argument navigation and action triggers. If any trailing text is
    // present (e.g. "/mcp servers timing out" or "/project files changed"),
    // do not match as a command so user messages are never dropped.
    if (args !== "") {
      return null;
    }
    return commandName as SlashCommandKey;
  }
  return null;
}

export const DEFAULT_SLASH_COMMANDS: PaletteItem[] = [
  {
    id: "cmd-model",
    kind: "slash",
    category: "command",
    title: "/model",
    subtitle: "Open models in Settings",
    insertText: "/model",
    icon: <Bot size={15} />,
  },
  {
    id: "cmd-clear",
    kind: "slash",
    category: "command",
    title: "/clear",
    subtitle: "Reset conversation and start fresh",
    insertText: "/clear",
    icon: <Trash2 size={15} />,
  },
  {
    id: "cmd-skill",
    kind: "slash",
    category: "command",
    title: "/skill",
    subtitle: "Attach an imported skill, or open Settings to import",
    insertText: "/skill",
    icon: <Sparkles size={15} />,
  },
  {
    id: "cmd-project",
    kind: "slash",
    category: "command",
    title: "/project",
    subtitle: "Browse and bind workspace files",
    insertText: "/project",
    icon: <FolderTree size={15} />,
  },
  {
    id: "cmd-mcp",
    kind: "slash",
    category: "command",
    title: "/mcp",
    subtitle: "Manage Model Context Protocol servers in Settings",
    insertText: "/mcp",
    icon: <Plug size={15} />,
  },
  {
    id: "cmd-help",
    kind: "slash",
    category: "command",
    title: "/help",
    subtitle: "Open keyboard shortcuts cheat sheet",
    insertText: "/help",
    icon: <HelpCircle size={15} />,
  },
];
