export const SECURITY_REPORT_ADDRESS = "support@xfalcon.ai";

export interface SecurityReportContext {
  applicationVersion: string;
  platform: "windows" | "macos" | "linux" | "unknown";
  now: Date;
}

const PLATFORM_NAMES: Record<SecurityReportContext["platform"], string> = {
  windows: "Windows",
  macos: "macOS",
  linux: "Linux",
  unknown: "Unknown",
};

/** A version reaches the subject and the body, so a line break in it must not
 * split either one. The app supplies this value, but it is cheap to be sure. */
function singleLine(value: string): string {
  return value.replace(/[\r\n]+/g, " ").trim();
}

export function securityReportSubject(applicationVersion: string): string {
  const version = singleLine(applicationVersion) || "unknown version";
  return `[URGENT SECURITY] xFalcon Desktop ${version} - needs immediate attention`;
}

/** Plain text with CRLF breaks, as RFC 6068 expects for a mail body. The blank
 * answer lines are part of the prompt: the reporter types under each heading. */
export function securityReportBody(context: SecurityReportContext): string {
  const version = singleLine(context.applicationVersion) || "unknown version";
  // `toISOString` always carries milliseconds; a report needs only seconds.
  const createdUtc = context.now.toISOString().replace(/\.\d{3}Z$/, "Z");

  return [
    "SECURITY REPORT - PLEASE TRIAGE IMMEDIATELY",
    "",
    "What happened (one or two sentences):",
    "",
    "",
    "Where in xFalcon (for example sign-in, an MCP server, file upload, updates, projects, reset):",
    "",
    "",
    "Steps to reproduce:",
    "1.",
    "2.",
    "3.",
    "",
    "Impact (what could someone do, or what was exposed):",
    "",
    "",
    "When it started or was noticed (date, time, time zone):",
    "",
    "",
    "Is it still happening? (yes / no / not sure):",
    "",
    "",
    "Who else knows about this? (nobody / my team / other):",
    "",
    "",
    "How to reach you (name, phone or another channel, best time):",
    "",
    "",
    "--- Added by xFalcon, please leave as is ---",
    `App: xFalcon Desktop ${version}`,
    `Platform: ${PLATFORM_NAMES[context.platform]}`,
    `Report created (UTC): ${createdUtc}`,
    "",
    "Please do NOT include passwords, API keys, access tokens or customer data in this email. If we need your diagnostics report we will ask for it.",
  ].join("\r\n");
}

/** The whole `mailto:` URL. Only subject and body are carried, percent-encoded,
 * so nothing a version string contains can add a header or a recipient. */
export function securityReportMailto(context: SecurityReportContext): string {
  const subject = securityReportSubject(context.applicationVersion);
  const body = securityReportBody(context).replace(/\r?\n/g, "\r\n");
  return `mailto:${SECURITY_REPORT_ADDRESS}?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`;
}
