import { useEffect, useRef } from "react";
import { PageTitle } from "../../ui/Heading";
import { settingsSectionElementId, type SettingsReveal } from "../../app/settingsCatalog";
import { SearchRevealProvider } from "../../lib/highlightedText";
import { scrollChildIntoContainer } from "../../lib/scrollInContainer";
import { AppearanceSection } from "./AppearanceSection";
import { ModelsConnectionsSection } from "./ModelsConnectionsSection";
import { AgentCapabilitiesSection } from "./AgentCapabilitiesSection";
import { ApplicationSection } from "./ApplicationSection";
import { SettingsPage } from "./SettingsLayout";

export interface SettingsViewProps {
  mcpRevision?: number;
  /**
   * Called whenever something that feeds `workspace_status` changes -- a
   * provider key, or the MCP registry. The shell re-reads it so the composer
   * reflects the change without a restart.
   */
  onWorkspaceChanged?: () => void;
  /** Scroll and mark this destination when the palette (or a slash command) opened Settings. */
  reveal?: SettingsReveal | null;
}

/**
 * Settings, in four titled sections (spec: settings-appearance).
 *
 * Ordered by how far a change reaches: Appearance changes only this window,
 * Models & connections change what a run can reach, Agent capabilities change
 * what it knows and how it answers, Application covers the app itself.
 * There is no separate tenant/workspace readout here -- the MCP servers
 * section already names every server this device talks to, and a raw tenant
 * hostname told a reader nothing they could act on.
 */
export function SettingsView({ onWorkspaceChanged, reveal = null, mcpRevision }: SettingsViewProps) {
  const pageRef = useRef<HTMLDivElement>(null);
  const revealSection = reveal?.section;
  const revealTargetId = reveal?.targetId;
  const revealToken = reveal?.token;

  useEffect(() => {
    if (revealSection === undefined || revealTargetId === undefined || revealToken === undefined) return;
    let cancelled = false;
    const delays = [0, 50, 160, 400];
    const timers: number[] = [];

    const jump = (): boolean => {
      const page = pageRef.current;
      if (!page) return false;
      const target =
        document.getElementById(revealTargetId) ?? document.getElementById(settingsSectionElementId(revealSection));
      if (!target || !page.contains(target)) return false;
      scrollChildIntoContainer(page, target);
      return true;
    };

    delays.forEach((delay) => {
      timers.push(
        window.setTimeout(() => {
          if (!cancelled) jump();
        }, delay),
      );
    });

    return () => {
      cancelled = true;
      timers.forEach((timer) => window.clearTimeout(timer));
    };
  }, [revealSection, revealTargetId, revealToken]);

  return (
    <SearchRevealProvider
      value={reveal ? { query: reveal.query, targetId: reveal.targetId, token: reveal.token } : null}
    >
      <SettingsPage ref={pageRef}>
        <div className="settings-page__intro">
          <PageTitle>Settings</PageTitle>
        </div>
        <AppearanceSection />
        <ModelsConnectionsSection mcpRevision={mcpRevision} onWorkspaceChanged={onWorkspaceChanged} />
        <AgentCapabilitiesSection />
        <ApplicationSection />
        <footer className="type-caption text-secondary">For further help, email support@xfalcon.ai.</footer>
      </SettingsPage>
    </SearchRevealProvider>
  );
}
