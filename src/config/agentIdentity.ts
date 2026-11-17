// ---------------------------------------------------------------------------
// Agent identity colors — per-entity DATA, not theme tokens.
//
// An agent's avatar color is an identity marker chosen per agent (like a
// member color in Slack/Linear). It is intentionally NOT part of the light/dark
// design-token system: it stays constant across themes. These literal Tailwind
// background utilities are the single config source for the identity palette;
// surfaces render them only as a small dot via `cn(..., agent.avatarColor)`.
//
// Excludes the brand accent (cyan) so identity dots never masquerade as the
// primary action color, and excludes neutral slate.
// ---------------------------------------------------------------------------

export interface AgentAvatarColor {
  /** Tailwind bg-* utility applied to the identity dot. */
  value: string;
  /** Human label for the picker. */
  label: string;
}

export const AGENT_AVATAR_COLORS: AgentAvatarColor[] = [
  { value: "bg-emerald-500", label: "Emerald" },
  { value: "bg-teal-500", label: "Teal" },
  { value: "bg-sky-500", label: "Sky" },
  { value: "bg-indigo-500", label: "Indigo" },
  { value: "bg-purple-500", label: "Purple" },
  { value: "bg-pink-500", label: "Pink" },
  { value: "bg-orange-500", label: "Orange" },
  { value: "bg-yellow-500", label: "Amber" },
];
