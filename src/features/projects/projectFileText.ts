import type { ProjectEntryKind, ProjectFileMetadataReason, ProjectSensitiveClass } from "../../lib/generated/contracts";

// Total `Record`s over the contract unions on purpose: a new arm in
// `crates/contracts` fails the build here instead of reaching a user as its own
// enum name.

/** Why a file's bytes were read but deliberately not shown as text. */
export const METADATA_ONLY_REASON: Record<ProjectFileMetadataReason, string> = {
  binary: "This file is not text, so there is nothing to show as text.",
  non_utf8_text: "This file is text, but not valid UTF-8, so it cannot be shown without corrupting it.",
  oversized: "This file is too large for xFalcon to read through, so not even its checksum was taken.",
};

/** What kind of secret a withheld path was classified as. */
export const SENSITIVE_CLASS_LABEL: Record<ProjectSensitiveClass, string> = {
  dot_env: "environment file",
  private_key_material: "private key material",
  cloud_or_kubernetes_credentials: "cloud or Kubernetes credentials",
  password_or_vault_export: "password manager or vault export",
  browser_profile_store: "browser profile store",
  ssh_or_gpg_material: "SSH or GPG material",
  token_or_config_store: "token or configuration store",
  configured: "a path this project marks as sensitive",
};

/** Entry kinds the explorer shows but will not open. */
export const UNOPENABLE_KIND_NOTE: Record<Extract<ProjectEntryKind, "symlink" | "other">, string> = {
  symlink: "Symbolic link — not followed",
  other: "Not a regular file",
};

/**
 * `text` with its line endings converted back to CRLF when `original` is CRLF
 * throughout; otherwise `text` as it is.
 *
 * A textarea reports every line break as a bare `\n`, whatever the text it was
 * given used, so saving what it holds would quietly turn a Windows file into a
 * Unix one and show every line as changed. A file that mixes both has no
 * convention to restore, so it is left as the editor reports it.
 */
export function inOriginalLineEndings(text: string, original: string): string {
  const crlf = original.match(/\r\n/g)?.length ?? 0;
  if (crlf === 0) return text;
  const lf = original.match(/\n/g)?.length ?? 0;
  if (lf !== crlf) return text;
  return text.replace(/\r?\n/g, "\r\n");
}

const BASE64_CHUNK_BYTES = 0x8000;

/**
 * UTF-8 text to base64, for a patch operation's `content_base64`.
 *
 * `btoa` throws on any code point above U+00FF, so the text is encoded to bytes
 * first and only those bytes reach it. Converted a chunk at a time because
 * `String.fromCharCode(...bytes)` overflows the call stack on a file of any real
 * size.
 */
export function utf8ToBase64(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += BASE64_CHUNK_BYTES) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + BASE64_CHUNK_BYTES));
  }
  return btoa(binary);
}

/**
 * A sentence for a `project_*` command failure.
 *
 * These commands reject with one of `projects`' own already-redacted `Display`
 * strings -- no absolute path, no filesystem layout, and no file content ever
 * reaches the frontend through one -- so the detail is worth showing. Anything
 * that is not a usable string is dropped rather than stringified at the user.
 */
export function describeProjectError(cause: unknown, fallback: string): string {
  const detail = typeof cause === "string" ? cause : cause instanceof Error ? cause.message : null;
  const trimmed = detail?.trim() ?? "";
  if (trimmed.length === 0) return fallback;
  const sentence = `${trimmed[0].toLocaleUpperCase()}${trimmed.slice(1)}`;
  return `${fallback} ${sentence.endsWith(".") ? sentence : `${sentence}.`}`;
}
