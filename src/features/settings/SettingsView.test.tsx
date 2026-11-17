import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SearchRevealProvider } from "../../lib/highlightedText";
import { SEARCH_REVEAL_MS } from "../../lib/searchReveal";
import { SettingsSection } from "./SettingsLayout";
import { SettingsView } from "./SettingsView";

vi.mock("./AppearanceSection", () => ({ AppearanceSection: () => null }));
vi.mock("./ModelsConnectionsSection", () => ({ ModelsConnectionsSection: () => null }));
vi.mock("./AgentCapabilitiesSection", () => ({
  AgentCapabilitiesSection: () => <section id="settings-section-agent-capabilities" />,
}));
vi.mock("./ApplicationSection", () => ({
  ApplicationSection: () => (
    <section id="settings-section-application">
      <h3>Application</h3>
      <div id="settings-support" />
      <div id="settings-reset" />
    </section>
  ),
}));

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("Settings order", () => {
  it("orders Agent capabilities before Application", () => {
    render(<SettingsView />);

    const sections = Array.from(document.querySelectorAll("[data-settings-page] section"));
    expect(sections.map((section) => section.id)).toEqual([
      "settings-section-agent-capabilities",
      "settings-section-application",
    ]);
  });

  it("reveals the Agent capabilities section as a reveal target", () => {
    render(
      <SettingsView
        reveal={{
          section: "agent-capabilities",
          targetId: "settings-section-agent-capabilities",
          query: "skills",
          token: 1,
        }}
      />,
    );

    expect(document.getElementById("settings-section-agent-capabilities")).toBeTruthy();
  });

  it("reveals Data & reset inside Application as a reveal target", () => {
    render(
      <SettingsView
        reveal={{
          section: "application",
          targetId: "settings-reset",
          query: "reset",
          token: 1,
        }}
      />,
    );

    expect(document.getElementById("settings-reset")).toBeTruthy();
  });
});

describe("SettingsView search jump", () => {
  it("does not call scrollIntoView on the window, which would drag the sidebar", () => {
    const intoView = vi.fn();
    HTMLElement.prototype.scrollIntoView = intoView;

    render(
      <SettingsView
        reveal={{
          section: "application",
          targetId: "settings-support",
          query: "support",
          token: 1,
        }}
      />,
    );

    expect(document.querySelector("[data-settings-page]")).toBeTruthy();
    expect(document.getElementById("settings-support")).toBeTruthy();
    expect(intoView).not.toHaveBeenCalled();
  });
});

describe("Settings support line", () => {
  it("renders the support line once, after the last section and outside any section", () => {
    render(<SettingsView />);

    const lines = screen.getAllByText("For further help, email support@xfalcon.ai.");
    expect(lines).toHaveLength(1);
    const sections = Array.from(document.querySelectorAll("[data-settings-page] section"));
    const lastSection = sections[sections.length - 1];
    expect(lastSection.compareDocumentPosition(lines[0]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(lines[0].closest("section")).toBeNull();
  });
});

describe("Settings section reveal", () => {
  it("marks the matched heading inside the jumped-to section", () => {
    render(
      <SearchRevealProvider value={{ query: "support", targetId: "settings-section-diagnostics" }}>
        <SettingsSection
          id="settings-section-diagnostics"
          title="Support"
          description="Create a private diagnostic file."
        >
          Body
        </SettingsSection>
      </SearchRevealProvider>,
    );

    const section = document.getElementById("settings-section-diagnostics");
    expect(section).toHaveAttribute("data-search-reveal");
    expect(section?.querySelector("mark.search-reveal-mark")).toHaveTextContent("Support");
  });

  it("drops the ring and marks after five seconds", () => {
    vi.useFakeTimers();
    render(
      <SearchRevealProvider value={{ query: "support", targetId: "settings-section-diagnostics", token: 1 }}>
        <SettingsSection
          id="settings-section-diagnostics"
          title="Support"
          description="Create a private diagnostic file."
        >
          Body
        </SettingsSection>
      </SearchRevealProvider>,
    );

    const section = document.getElementById("settings-section-diagnostics");
    expect(section).toHaveAttribute("data-search-reveal");

    act(() => {
      vi.advanceTimersByTime(SEARCH_REVEAL_MS);
    });

    expect(section).not.toHaveAttribute("data-search-reveal");
    expect(section?.querySelector("mark.search-reveal-mark")).toBeNull();
  });

  it("flashes again when the jump token changes", () => {
    vi.useFakeTimers();
    const { rerender } = render(
      <SearchRevealProvider value={{ query: "support", targetId: "settings-section-diagnostics", token: 1 }}>
        <SettingsSection id="settings-section-diagnostics" title="Support">
          Body
        </SettingsSection>
      </SearchRevealProvider>,
    );

    act(() => {
      vi.advanceTimersByTime(SEARCH_REVEAL_MS);
    });
    expect(document.getElementById("settings-section-diagnostics")).not.toHaveAttribute("data-search-reveal");

    rerender(
      <SearchRevealProvider value={{ query: "support", targetId: "settings-section-diagnostics", token: 2 }}>
        <SettingsSection id="settings-section-diagnostics" title="Support">
          Body
        </SettingsSection>
      </SearchRevealProvider>,
    );
    expect(document.getElementById("settings-section-diagnostics")).toHaveAttribute("data-search-reveal");
  });
});
