import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppBootstrap } from "./lib/generated/contracts";
import App from "./App";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => vi.fn()),
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn(async () => "0.5.1"),
}));

const bootstrap: AppBootstrap = {
  version: 1,
  application_version: "0.1.0",
  platform: "windows",
  session: {
    status: "authenticated",
    version: 1,
    identity: {
      version: 1,
      user_id: "user-test",
      tenant_id: "tenant-test",
      display_name: "Test User",
      email: null,
    },
    expires_at_ms: Date.now() + 3_600_000,
  },
};

afterEach(() => {
  cleanup();
  invokeMock.mockReset();
});

const workspaceStatus = {
  version: 1,
  tenant_host: "tenant.example",
  mcp_resource_url: "https://tenant.example/mcp",
  mcp_servers: [],
  models_available: 1,
  default_model_id: "gpt-test",
  default_provider: "openai",
  chat_ready: true,
  setup_note: "Bundled MCP is automatic.",
} as const;

describe("native application bootstrap", () => {
  it("shows native readiness and keeps local-data clear out of the account menu", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "app_bootstrap") return bootstrap;
      if (command === "workspace_status") return workspaceStatus;
      if (command === "project_list") return { version: 1, projects: [] };
      throw new Error(`Unexpected command: ${command}`);
    });

    const user = userEvent.setup();
    render(<App />);

    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("Starting xFalcon");
    // `.app-startup h1`/`p` used to hand-duplicate the type-role values at
    // (0,1,1) specificity, which beat any `.type-*` role class an element
    // carried (0,1,0) regardless of source order, and supplied no font-weight
    // at all -- Tailwind's own heading reset left every startup `<h1>` at
    // `font-weight: inherit` with nothing above it to set one. `PageTitle`
    // and an explicit `type-body` now own the look instead of the container.
    expect(status).toHaveClass("type-body", "text-secondary");
    expect(screen.getByRole("heading", { name: "xFalcon Desktop" })).toHaveClass("type-title", "font-semibold");
    expect(await screen.findByRole("heading", { name: "What can I help you with, Test User?" })).toBeInTheDocument();
    expect(screen.queryByText("xFalcon is ready")).not.toBeInTheDocument();
    expect(screen.getByText("Test User")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("app_bootstrap");

    await user.click(screen.getByText("Test User"));
    expect(screen.getByRole("menuitem", { name: "Sign out" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Clear local data" })).not.toBeInTheDocument();
    expect(screen.queryByText("Local privacy")).not.toBeInTheDocument();
  });

  it("shows a redacted bootstrap error and retries the native boundary", async () => {
    invokeMock
      .mockRejectedValueOnce(new Error("native bootstrap unavailable at C:\\Users\\person\\sensitive"))
      .mockImplementation(async (command: string) => {
        if (command === "app_bootstrap") return bootstrap;
        if (command === "workspace_status") return workspaceStatus;
        if (command === "project_list") return { version: 1, projects: [] };
        throw new Error(`Unexpected command: ${command}`);
      });

    const user = userEvent.setup();
    render(<App />);

    expect(await screen.findByRole("alert")).toHaveTextContent("Unable to start xFalcon");
    expect(screen.getByRole("alert")).toHaveTextContent("The secure native runtime could not be reached.");
    expect(screen.getByRole("alert")).not.toHaveTextContent("C:\\Users\\person\\sensitive");
    expect(screen.getByRole("heading", { name: "Unable to start xFalcon" })).toHaveClass("type-title", "font-semibold");
    expect(screen.queryByRole("button", { name: "Clear local data" })).not.toBeInTheDocument();
    expect(screen.queryByText("MCP")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("heading", { name: "What can I help you with, Test User?" })).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("app_bootstrap");
    expect(invokeMock).toHaveBeenCalledWith("workspace_status");
  });

  it("does not offer local-data cleanup from the account menu", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "app_bootstrap") return bootstrap;
      if (command === "workspace_status") return workspaceStatus;
      if (command === "project_list") return { version: 1, projects: [] };
      throw new Error(`Unexpected command: ${command}`);
    });

    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole("heading", { name: "What can I help you with, Test User?" });
    await user.click(screen.getByRole("button", { name: "Account and app menu" }));
    expect(screen.queryByRole("menuitem", { name: "Clear local data" })).not.toBeInTheDocument();
  });
});
