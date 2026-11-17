import React from "react";
import type { AppearanceSettings } from "../../lib/generated/contracts";
import { chooseAppearance, useAppearance, withShowFollowUps, withShowReasoning } from "../../app/appearance";
import { Switch } from "../../ui/Switch";
import { toast } from "../../ui/toastStore";
import { APPEARANCE_COPY } from "./settingsCopy";
import { settingsSectionElementId } from "../../app/settingsCatalog";
import { SettingsGroup, SettingsRow, SettingsSection } from "./SettingsLayout";
import { SkillsSection } from "./SkillsSection";

/**
 * Model reasoning, follow-up suggestions and the skill library (spec:
 * chat-reasoning-panel, user-skills).
 *
 * Every switch applies on click and persists behind that. A failed write is
 * surfaced as a toast and the applied setting is left standing: the user can
 * see what they chose, and telling them it was saved when it was not is the
 * worse failure.
 *
 * The values are read from the appearance store rather than fetched here. The
 * transcript reads `show_reasoning` too, and a second copy loaded on mount is a
 * second source of truth that can disagree with what the rest of the app shows.
 */
export function AgentCapabilitiesSection() {
  const appearance = useAppearance();

  const choose = React.useCallback((next: AppearanceSettings) => {
    chooseAppearance(next).catch(() => {
      toast.error(APPEARANCE_COPY.saveFailed);
    });
  }, []);

  return (
    <SettingsSection id={settingsSectionElementId("agent-capabilities")} title="Agent capabilities">
      <SettingsGroup aria-label="Agent capability settings">
        <SettingsRow
          id="settings-show-reasoning-row"
          label={APPEARANCE_COPY.showReasoningLabel}
          hint={APPEARANCE_COPY.showReasoningHint}
          labelId="settings-show-reasoning-label"
        >
          <Switch
            aria-labelledby="settings-show-reasoning-label"
            checked={appearance.show_reasoning}
            onChange={(show_reasoning) => choose(withShowReasoning(appearance, show_reasoning))}
          />
        </SettingsRow>
        <SettingsRow
          id="settings-show-follow-ups-row"
          label={APPEARANCE_COPY.showFollowUpsLabel}
          hint={APPEARANCE_COPY.showFollowUpsHint}
          labelId="settings-show-follow-ups-label"
        >
          <Switch
            aria-labelledby="settings-show-follow-ups-label"
            checked={appearance.show_follow_ups}
            onChange={(show_follow_ups) => choose(withShowFollowUps(appearance, show_follow_ups))}
          />
        </SettingsRow>
        <SkillsSection />
      </SettingsGroup>
    </SettingsSection>
  );
}
