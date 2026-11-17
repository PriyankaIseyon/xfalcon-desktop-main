import { afterEach, describe, expect, it } from "vitest";
import type { SessionProjection } from "../../lib/generated/contracts";
import { resetAuthSessionForTests, setAuthSession } from "./authStore";
import { clearSignedOutNotice, getSignedOutNotice, setSignedOutNotice } from "./signedOutNotice";

const AUTHENTICATED: SessionProjection = {
  status: "authenticated",
  version: 1,
  identity: { version: 1, user_id: "user-1", tenant_id: "tenant-1", display_name: "Test User", email: null },
  expires_at_ms: Date.now() + 60_000,
};

afterEach(() => {
  resetAuthSessionForTests();
});

describe("signedOutNotice", () => {
  it("holds the text it is given until it is cleared", () => {
    expect(getSignedOutNotice()).toBeNull();

    setSignedOutNotice("Signed out after 12 hours of inactivity. Sign in to continue.");
    expect(getSignedOutNotice()).toBe("Signed out after 12 hours of inactivity. Sign in to continue.");

    clearSignedOutNotice();
    expect(getSignedOutNotice()).toBeNull();
  });

  it("is cleared when a new session is authenticated", () => {
    setSignedOutNotice("Signed out after 12 hours of inactivity. Sign in to continue.");

    setAuthSession(AUTHENTICATED);

    expect(getSignedOutNotice()).toBeNull();
  });

  it("survives an unauthenticated session update", () => {
    setSignedOutNotice("Signed out after 12 hours of inactivity. Sign in to continue.");

    setAuthSession({ status: "unauthenticated", version: 1 });

    expect(getSignedOutNotice()).toBe("Signed out after 12 hours of inactivity. Sign in to continue.");
  });
});
