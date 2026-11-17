import { McpServersSection } from "./McpServersSection";
import { ModelsSection } from "./ModelsSection";
import { PlanProvidersSection } from "./PlanProvidersSection";
import { ProvidersSection } from "./ProvidersSection";
import { SettingsGroup, SettingsSection } from "./SettingsLayout";
import { settingsSectionElementId } from "../../app/settingsCatalog";

export interface ModelsConnectionsSectionProps {
  mcpRevision?: number;
  onWorkspaceChanged?: () => void;
}

export function ModelsConnectionsSection({ onWorkspaceChanged, mcpRevision }: ModelsConnectionsSectionProps) {
  return (
    <SettingsSection id={settingsSectionElementId("models-connections")} title="Models & connections">
      <SettingsGroup aria-label="Model and connection settings">
        <ModelsSection />
        <ProvidersSection onCredentialsChanged={onWorkspaceChanged} />
        <PlanProvidersSection onWorkspaceChanged={onWorkspaceChanged} />
        <McpServersSection revision={mcpRevision} onRegistryChanged={onWorkspaceChanged} />
      </SettingsGroup>
    </SettingsSection>
  );
}
