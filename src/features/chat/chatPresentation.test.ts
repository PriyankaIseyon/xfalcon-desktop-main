import { describe, expect, it } from "vitest";
import { chatErrorCode, chatErrorIsTransient, describeChatError } from "./chatPresentation";

/**
 * The wire shape a rejected chat command actually has. Tauri serializes the
 * error *value*, so `ChatOperationError` arrives as a plain object.
 */
function rejection(code: string, message: string) {
  return { version: 1, code, message };
}

const BRIDGE_FALLBACK = "Could not retry the response. The secure runtime did not respond.";

describe("chat error presentation", () => {
  it("renders the workspace's own reason rather than the caller's fallback", () => {
    expect(
      describeChatError(
        rejection("temporarily_unavailable", "The workspace is not responding right now. Try again in a moment."),
        BRIDGE_FALLBACK,
      ),
    ).toBe("The workspace is not responding right now. Try again in a moment.");
  });

  // The defect this module exists for: a workspace that was mid-deploy reported
  // a failure of the *local* runtime, which is a different machine and a
  // different fix. Only an unreadable rejection may claim that now.
  it("never blames the secure runtime for a typed workspace failure", () => {
    for (const code of ["temporarily_unavailable", "service_unavailable", "session_expired", "too_many_runs"]) {
      expect(describeChatError(rejection(code, "Something specific happened."), BRIDGE_FALLBACK)).not.toBe(
        BRIDGE_FALLBACK,
      );
    }
  });

  it("falls back only when the bridge itself returned nothing readable", () => {
    expect(describeChatError(new Error("boom"), BRIDGE_FALLBACK)).toBe(BRIDGE_FALLBACK);
    expect(describeChatError(undefined, BRIDGE_FALLBACK)).toBe(BRIDGE_FALLBACK);
    expect(describeChatError("plain string", BRIDGE_FALLBACK)).toBe(BRIDGE_FALLBACK);
    expect(describeChatError({ code: "not_a_chat_code" }, BRIDGE_FALLBACK)).toBe(BRIDGE_FALLBACK);
  });

  // A code with no prose is still a code: the surface must not fall through to
  // a sentence about the wrong machine just because the message was empty.
  it("uses the code's own sentence when the message is blank", () => {
    expect(describeChatError(rejection("too_many_runs", "   "), BRIDGE_FALLBACK)).toBe(
      "Too many active runs. Cancel one and try again.",
    );
  });

  it("never renders a machine token at the reader", () => {
    const rendered = describeChatError(rejection("not_found", "That conversation no longer exists."), BRIDGE_FALLBACK);
    expect(rendered).not.toContain("conversation_not_found");
    expect(rendered).not.toContain("_");
  });

  it("exposes the code for the decisions prose cannot carry", () => {
    expect(chatErrorCode(rejection("not_found", "That conversation no longer exists."))).toBe("not_found");
    expect(chatErrorCode(new Error("conversation_not_found"))).toBeNull();
  });

  // Retrying is honest advice for exactly one shape. Offering it for a contract
  // violation spends the reader's time on a request that cannot win.
  it("marks only an outage as worth waiting out", () => {
    expect(chatErrorIsTransient(rejection("temporarily_unavailable", "x"))).toBe(true);
    expect(chatErrorIsTransient(rejection("service_unavailable", "x"))).toBe(false);
    expect(chatErrorIsTransient(rejection("not_found", "x"))).toBe(false);
    expect(chatErrorIsTransient(new Error("boom"))).toBe(false);
  });
});
