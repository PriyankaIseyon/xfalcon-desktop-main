import { beforeEach, describe, expect, it } from "vitest";
import { getCanvasState, openCanvasArtifact, type TableArtifact } from "../canvas/canvasStore";
import { getChatSnapshot, setChatStoreForTests } from "../chat/chatStore";
import { applyDatasetList, getDatasetsSnapshot } from "../datasets/datasetsStore";
import { applyReportList, getReportsSnapshot } from "../reports/reportsStore";
import { getAuthAccountGeneration, getAuthSession, resetAuthSessionForTests, setAuthSession } from "./authStore";
import type { IdentityProjection, SessionProjection } from "../../lib/generated/contracts";

/**
 * Proves the cross-account state defect the code audit found is closed:
 * signing in as a *different* account must not let that account's shell
 * mount onto anything the previous account left in `chatStore`,
 * `datasetsStore`, or `canvasStore` — the same guarantee `reportsStore`
 * already had. Each assertion below was confirmed to fail (the stale value
 * survives) against `authStore.ts` before `resetAccountScopedStores` existed.
 */

function identity(overrides: Partial<IdentityProjection> = {}): IdentityProjection {
  return {
    version: 1,
    user_id: "user-1",
    tenant_id: "tenant-1",
    display_name: "Ada Lovelace",
    email: "ada@example.com",
    ...overrides,
  };
}

function authenticated(overrides: Partial<IdentityProjection> = {}): SessionProjection {
  return { status: "authenticated", version: 1, identity: identity(overrides), expires_at_ms: 1_000 };
}

const unauthenticated: SessionProjection = { status: "unauthenticated", version: 1 };

function seedAccountScopedStores(): void {
  setChatStoreForTests({
    conversationId: "conv-1",
    title: "Q3 pipeline review",
    messages: [{ id: "m1", role: "user", text: "previous account's message", sequence: BigInt(1), toolCalls: [] }],
    conversations: [
      {
        conversation_id: "conv-1",
        title: "Q3 pipeline review",
        project_id: null,
        pinned_at_ms: null,
        updated_at_ms: 1,
      } as never,
    ],
  });
  applyDatasetList([{ id: "ds-1", name: "previous account's dataset" } as never], true, false);
  const artifact: TableArtifact = {
    id: "art-1",
    type: "table",
    title: "previous account's table",
    columns: ["a"],
    rows: [["1"]],
  };
  openCanvasArtifact(artifact);
  applyReportList([{ id: "r-1", name: "previous account's report" } as never]);
}

beforeEach(() => {
  resetAuthSessionForTests();
});

describe("account-scoped store reset", () => {
  it("clears chat, dataset, canvas, and report state when a different account signs in", () => {
    setAuthSession(authenticated());
    seedAccountScopedStores();

    expect(getChatSnapshot().conversationId).toBe("conv-1");
    expect(getDatasetsSnapshot().datasets).toHaveLength(1);
    expect(getCanvasState().tabs).toHaveLength(1);
    expect(getReportsSnapshot().reports).toHaveLength(1);

    setAuthSession(authenticated({ tenant_id: "tenant-2", user_id: "user-2" }));

    expect(getChatSnapshot().conversationId).toBeNull();
    expect(getChatSnapshot().messages).toEqual([]);
    expect(getChatSnapshot().conversations).toEqual([]);
    expect(getDatasetsSnapshot().datasets).toEqual([]);
    expect(getCanvasState().tabs).toEqual([]);
    expect(getCanvasState().isOpen).toBe(false);
    expect(getReportsSnapshot().reports).toEqual([]);
  });

  it("clears the same state on sign-out, before the next sign-in can render", () => {
    setAuthSession(authenticated());
    seedAccountScopedStores();

    setAuthSession(unauthenticated);

    expect(getChatSnapshot().conversationId).toBeNull();
    expect(getChatSnapshot().messages).toEqual([]);
    expect(getDatasetsSnapshot().datasets).toEqual([]);
    expect(getCanvasState().tabs).toEqual([]);
    expect(getReportsSnapshot().reports).toEqual([]);
  });

  it("does not wipe an authenticated account's own state on a same-account token refresh", () => {
    setAuthSession(authenticated());
    seedAccountScopedStores();
    const generation = getAuthAccountGeneration();

    // Same tenant + user, only the token's expiry moved — a session refresh,
    // not an account change.
    setAuthSession(authenticated());
    setAuthSession({ status: "authenticated", version: 1, identity: identity(), expires_at_ms: 2_000 });

    expect(getChatSnapshot().conversationId).toBe("conv-1");
    expect(getDatasetsSnapshot().datasets).toHaveLength(1);
    expect(getCanvasState().tabs).toHaveLength(1);
    expect(getReportsSnapshot().reports).toHaveLength(1);
    expect(getAuthAccountGeneration()).toBe(generation);
  });

  it("invalidates asynchronous work when the account changes", () => {
    setAuthSession(authenticated());
    const accountOneGeneration = getAuthAccountGeneration();

    setAuthSession(authenticated({ tenant_id: "tenant-2", user_id: "user-2" }));

    expect(getAuthAccountGeneration()).toBeGreaterThan(accountOneGeneration);
  });

  it("still updates the session projection itself alongside the reset", () => {
    const next = authenticated({ tenant_id: "tenant-2", user_id: "user-2" });
    setAuthSession(next);
    expect(getAuthSession()).toEqual(next);
  });
});
