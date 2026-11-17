# Security Policy

## Supported versions

xFalcon Desktop is under active MVP development. No production version is
currently declared generally available or supported, and repository version
numbers must not be interpreted as release-support commitments.

Before general availability, this section will identify supported release
channels and security-update windows. Until then, security fixes are evaluated
against the active development line, and no guarantee is made that an
unreleased build is safe for production data.

## Reporting a vulnerability

Do not open a public issue containing vulnerability details.

Use GitHub's private vulnerability reporting flow for this repository:
[report a vulnerability privately](https://github.com/iSeyon-Analytics/xfalcon-desktop/security/advisories/new).
If that flow is unavailable, open a public issue containing only a request for a
private maintainer contact. Do not include exploit details, secrets, personal
data, customer identifiers, tokens, file contents, or production endpoints in
that issue.

A useful private report includes:

- affected commit or build identifier and platform;
- vulnerability class and security impact;
- minimal, non-destructive reproduction steps;
- whether the issue is known to be actively exploited;
- sanitized logs, screenshots, or proof-of-concept material; and
- a safe method for follow-up contact.

Never use real secrets or customer data in a report. Replace them with synthetic
values and remove authorization headers, cookies, OAuth codes, provider keys,
MCP credentials, message/file content, ciphertext, and tenant/user identifiers.

## Response targets

The following are coordination targets, not guarantees or service-level
commitments:

- acknowledge a complete private report within 3 business days;
- provide an initial severity and scope assessment within 10 business days;
- provide a status update at least every 10 business days while remediation is
  active; and
- coordinate disclosure after a fix or mitigation is available, accounting for
  active exploitation and downstream dependencies.

Complex issues, inherited platform controls, release-signing dependencies, and
multi-party remediation can require more time. The security owner will explain
material changes to the target timeline through the private report.

## Scope

In scope:

- source code, Tauri capabilities, configuration, packaging, and release
  workflows in this repository;
- Rust/React boundaries, local persistence, project-file access, provider
  credential handling, model streaming, MCP and skills, uploads, artifacts, and
  updater behavior implemented by xFalcon Desktop; and
- dependency or build-chain weaknesses that affect distributed desktop
  artifacts.

DA, Casdoor, object storage, model providers, GitHub, and signing/KMS services
are independently operated dependencies. Reports showing an xFalcon Desktop
integration weakness remain in scope; vulnerabilities solely within those
services should also be reported to their operator. XF Web is not a Desktop
runtime dependency.

Out of scope without prior written authorization:

- denial-of-service or load testing against production services;
- social engineering, phishing, physical access, or employee targeting;
- access to or modification of data belonging to another person or tenant;
- persistence, lateral movement, malware deployment, or destructive testing;
- automated scanning that materially degrades service; and
- testing third-party services beyond what is necessary to demonstrate a
  Desktop integration issue.

## Safe harbor

We support good-faith research that follows this policy, avoids privacy harm and
service disruption, uses the minimum access necessary, stops when sensitive data
is encountered, and reports findings promptly through the private channel. For
research meeting those conditions, xFalcon will treat the activity as
authorized for the purpose of investigating the reported issue and will not
initiate legal action solely because of that research.

This safe-harbor statement cannot authorize testing of third-party systems or
override applicable law. If you are uncertain whether an action is permitted,
ask through the private reporting channel before proceeding.

## Security architecture

- [Security model, controls, and enforcement map](docs/security.md)
- [Threat model](docs/threat-model.md)
- [Control matrix and evidence status](docs/compliance.md)
- [Architecture, trust boundaries, and data flows](docs/architecture.md)
- [Decision record](docs/decisions.md)
