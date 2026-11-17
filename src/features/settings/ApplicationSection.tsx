import { settingsSectionElementId } from "../../app/settingsCatalog";
import { DataResetSection } from "./DataResetSection";
import { DiagnosticsSection } from "./DiagnosticsSection";
import { SettingsGroup, SettingsSection } from "./SettingsLayout";
import { UpdateSection } from "./UpdateSection";

/** The app itself: updates, support and reset (spec: settings-application). */
export function ApplicationSection() {
  return (
    <SettingsSection id={settingsSectionElementId("application")} title="Application">
      <SettingsGroup aria-label="Application settings">
        <UpdateSection />
        <DiagnosticsSection />
        <DataResetSection />
      </SettingsGroup>
    </SettingsSection>
  );
}
