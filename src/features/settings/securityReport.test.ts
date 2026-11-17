import { describe, expect, it } from "vitest";
import {
  SECURITY_REPORT_ADDRESS,
  securityReportBody,
  securityReportMailto,
  securityReportSubject,
} from "./securityReport";

const now = new Date("2026-10-13T07:17:34.123Z");
const context = { applicationVersion: "0.5.37", platform: "macos" as const, now };

const HEADINGS = [
  "SECURITY REPORT - PLEASE TRIAGE IMMEDIATELY",
  "What happened (one or two sentences):",
  "Where in xFalcon (for example sign-in, an MCP server, file upload, updates, projects, reset):",
  "Steps to reproduce:",
  "Impact (what could someone do, or what was exposed):",
  "When it started or was noticed (date, time, time zone):",
  "Is it still happening? (yes / no / not sure):",
  "Who else knows about this? (nobody / my team / other):",
  "How to reach you (name, phone or another channel, best time):",
  "--- Added by xFalcon, please leave as is ---",
  "Please do NOT include passwords, API keys, access tokens or customer data in this email.",
];

describe("securityReportSubject", () => {
  it("names the urgency and the version", () => {
    expect(securityReportSubject("0.5.37")).toBe(
      "[URGENT SECURITY] xFalcon Desktop 0.5.37 - needs immediate attention",
    );
  });

  it("falls back to a readable placeholder when the version is empty", () => {
    expect(securityReportSubject("")).toBe(
      "[URGENT SECURITY] xFalcon Desktop unknown version - needs immediate attention",
    );
  });
});

describe("securityReportBody", () => {
  const body = securityReportBody(context);

  it("lists the headings in the order the reporter answers them", () => {
    const indexes = HEADINGS.map((heading) => body.indexOf(heading));
    indexes.forEach((index) => expect(index).toBeGreaterThanOrEqual(0));
    expect([...indexes].sort((a, b) => a - b)).toEqual(indexes);
  });

  it("fills in the app, platform and UTC creation time", () => {
    expect(body).toContain("App: xFalcon Desktop 0.5.37");
    expect(body).toContain("Platform: macOS");
    expect(body).toContain("Report created (UTC): 2026-10-13T07:17:34Z");
  });

  it("warns against secrets and customer data", () => {
    expect(body).toContain("Please do NOT include passwords, API keys, access tokens or customer data");
  });

  it("uses CRLF line breaks only", () => {
    expect(body).toContain("\r\n");
    expect(body.replace(/\r\n/g, "")).not.toMatch(/\n/);
  });
});

describe("securityReportMailto", () => {
  const url = securityReportMailto(context);
  const query = new URL(url).searchParams;

  it("addresses support and carries only subject and body", () => {
    expect(url.startsWith(`mailto:${SECURITY_REPORT_ADDRESS}?subject=`)).toBe(true);
    expect([...query.keys()]).toEqual(["subject", "body"]);
  });

  it("round-trips subject and body byte for byte", () => {
    expect(query.get("subject")).toBe(securityReportSubject("0.5.37"));
    expect(query.get("body")).toBe(securityReportBody(context).replace(/\r?\n/g, "\r\n"));
  });

  it("keeps characters that are special in URLs inside the body", () => {
    const hostile = { ...context, applicationVersion: "0.5 & 1=1 #100% +x" };
    const roundTrip = new URL(securityReportMailto(hostile)).searchParams;
    expect(roundTrip.get("body")).toContain("App: xFalcon Desktop 0.5 & 1=1 #100% +x");
    expect(roundTrip.get("subject")).toBe(securityReportSubject("0.5 & 1=1 #100% +x"));
  });

  it("stays under 2000 characters", () => {
    expect(url.length).toBeLessThan(2000);
  });

  it("cannot be steered into adding a recipient or header through the version", () => {
    const hostile = { ...context, applicationVersion: "1.0&cc=evil@x.test" };
    const hostileUrl = securityReportMailto(hostile);
    const params = new URL(hostileUrl).searchParams;
    expect(params.get("cc")).toBeNull();
    expect(params.getAll("subject")).toHaveLength(1);
    expect(params.get("subject")).toContain("1.0&cc=evil@x.test");
    expect(hostileUrl.startsWith(`mailto:${SECURITY_REPORT_ADDRESS}?subject=`)).toBe(true);
  });
});
