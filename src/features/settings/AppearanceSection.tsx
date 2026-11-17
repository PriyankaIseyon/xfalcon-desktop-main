import React from "react";
import type { AppearanceSettings, ReadingSize, ThemePreference } from "../../lib/generated/contracts";
import { chooseAppearance, useAppearance, withReadingSize, withTheme } from "../../app/appearance";
import { toast } from "../../ui/toastStore";
import { APPEARANCE_COPY, READING_SIZE_OPTIONS, THEME_OPTIONS } from "./settingsCopy";
import { SettingsSegmentedControl } from "./SettingsControls";
import { settingsSectionElementId } from "../../app/settingsCatalog";
import { SettingsGroup, SettingsRow, SettingsSection } from "./SettingsLayout";

/**
 * Theme and reading size (spec: settings-appearance). Reasoning and follow-up
 * suggestions live in AgentCapabilitiesSection.
 *
 * Every control applies on click and persists behind that, so the app changes
 * under the pointer rather than after a round trip. A failed write is
 * surfaced as a toast and the applied look is left standing: the user can see
 * what they chose, and telling them it was saved when it was not is the worse
 * failure.
 *
 * The value is read from the appearance store rather than fetched here. This
 * section is no longer the only reader — the transcript reads
 * `show_reasoning` — and a second copy loaded on mount is a second source of
 * truth that can disagree with the one the rest of the app is rendering.
 */
export function AppearanceSection() {
  const appearance = useAppearance();

  const choose = React.useCallback((next: AppearanceSettings) => {
    chooseAppearance(next).catch(() => {
      toast.error(APPEARANCE_COPY.saveFailed);
    });
  }, []);

  return (
    <SettingsSection id={settingsSectionElementId("appearance")} title={APPEARANCE_COPY.title}>
      <SettingsGroup aria-label="Appearance preferences">
        <SettingsRow label={APPEARANCE_COPY.themeLabel} hint={APPEARANCE_COPY.themeHint}>
          <SettingsSegmentedControl<ThemePreference>
            aria-label={APPEARANCE_COPY.themeLabel}
            options={THEME_OPTIONS}
            value={appearance.theme}
            onChange={(theme) => choose(withTheme(appearance, theme))}
          />
        </SettingsRow>

        <SettingsRow label={APPEARANCE_COPY.readingSizeLabel} hint={APPEARANCE_COPY.readingSizeHint}>
          <SettingsSegmentedControl<ReadingSize>
            aria-label={APPEARANCE_COPY.readingSizeLabel}
            options={READING_SIZE_OPTIONS.map((option) => ({
              ...option,
              label: (
                <span className="reading-size-preview" data-reading-size={option.value}>
                  {option.label}
                </span>
              ),
            }))}
            value={appearance.reading_size}
            onChange={(reading_size) => choose(withReadingSize(appearance, reading_size))}
          />
        </SettingsRow>
      </SettingsGroup>
    </SettingsSection>
  );
}
