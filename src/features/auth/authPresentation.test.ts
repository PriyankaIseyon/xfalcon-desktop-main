import { describe, expect, it } from "vitest";
import type { AuthLoginErrorCode } from "../../lib/generated/contracts";
import { describeLoginError, loginErrorOf } from "./authPresentation";

const CODES: AuthLoginErrorCode[] = [
  "already_authenticating",
  "not_configured",
  "browser_unavailable",
  "discovery_failed",
  "issuer_mismatch",
  "discovery_metadata_invalid",
  "loopback_unavailable",
  "callback_invalid",
  "callback_timed_out",
  "redirect_not_registered",
  "token_exchange_failed",
  "identity_failed",
  "device_clock_skew",
  "cancelled",
  "account_bind_failed",
];

describe("describeLoginError", () => {
  it("says something distinct for every code", () => {
    const said = CODES.map(describeLoginError);
    expect(said.every((line) => line.length > 0)).toBe(true);
    expect(new Set(said).size).toBe(CODES.length);
  });

  it("sends a person to their administrator only for what an administrator holds", () => {
    // A busy port and an abandoned browser tab are the person's own to fix;
    // pointing them at an administrator wastes both their time.
    expect(describeLoginError("loopback_unavailable")).not.toContain("administrator");
    expect(describeLoginError("callback_timed_out")).not.toContain("administrator");
    expect(describeLoginError("already_authenticating")).not.toContain("administrator");
    // A wrong clock on this machine is not something an administrator can fix.
    expect(describeLoginError("device_clock_skew")).not.toContain("administrator");
    expect(describeLoginError("device_clock_skew")).toContain("date and time");
    expect(describeLoginError("not_configured")).toContain("administrator");
    expect(describeLoginError("issuer_mismatch")).toContain("administrator");
    expect(describeLoginError("redirect_not_registered")).toContain("administrator");
  });
});

describe("loginErrorOf", () => {
  it("reads the code a refused command rejected with", () => {
    expect(loginErrorOf({ version: 1, code: "cancelled" })).toBe("cancelled");
  });

  it("reports nothing for a rejection that is not one of ours", () => {
    // The boundary itself failing must not put its own text on screen: it can
    // carry a path or an internal URL.
    expect(loginErrorOf(new Error("auth is not configured at https://issuer.example/internal"))).toBeNull();
    expect(loginErrorOf("some string")).toBeNull();
    expect(loginErrorOf(null)).toBeNull();
    expect(loginErrorOf({ code: 7 })).toBeNull();
  });
});
