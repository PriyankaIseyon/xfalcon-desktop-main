import type { ProjectDocumentRead, ProjectFileStat } from "../../lib/generated/contracts";
import {
  PROJECT_READ_RANGE_MAX_BYTES,
  projectDocumentRead,
  projectFileReadBytes,
  projectFileStat,
} from "../../lib/native";
import { splitLines } from "./fileLines";
import { PROJECT_FILE_LIMITS } from "./projectFileLimits";

export type TextStat = Extract<ProjectFileStat, { kind: "text" }>;

type FileVersion = ProjectFileStat | ProjectDocumentRead;

/** What the view holds for one file (spec: project-file-editing). */
export type LoadedFile =
  /** Bounded, read-only text extracted from an Office binary. */
  | { mode: "document"; stat: ProjectDocumentRead; text: string }
  /** Read whole, and valid UTF-8 throughout. */
  | { mode: "whole"; stat: TextStat; text: string }
  /** Too large to hold whole; read a window at a time. */
  | { mode: "windowed"; stat: TextStat }
  /** Not read as text: binary, another encoding, sensitive, or past the cap.
   * `stat` is the backend's answer, or a text stat whose bytes turned out not
   * to be UTF-8 past the sniffed prefix. */
  | { mode: "opaque"; stat: ProjectFileStat };

/** The stat a text file's bytes were read at, compared to decide staleness:
 * any change in size or modified time means the file on screen is old. */
export function sameVersion(left: FileVersion, right: FileVersion): boolean {
  const leftDocument = "text" in left;
  const rightDocument = "text" in right;
  if (leftDocument || rightDocument) {
    const document = (leftDocument ? left : right) as ProjectDocumentRead;
    const stat = leftDocument ? right : left;
    if (!("kind" in stat) || !("size_bytes" in stat) || !("modified_ms" in stat)) return false;
    if (
      document.path !== stat.path ||
      document.size_bytes !== stat.size_bytes ||
      document.modified_ms !== stat.modified_ms
    ) {
      return false;
    }
    return stat.kind === "text" || stat.kind === "not_text";
  }
  if (left.kind !== right.kind || left.path !== right.path) return false;
  if (left.kind === "text" && right.kind === "text") {
    return left.size_bytes === right.size_bytes && left.modified_ms === right.modified_ms;
  }
  return JSON.stringify(left) === JSON.stringify(right);
}

/** Word and PowerPoint files, whose text the backend can read. */
export function isOfficePath(path: string): boolean {
  return /\.(doc|docx|ppt|pptx)$/i.test(path);
}

/** Every file the viewer can turn into a PDF preview: Word, PowerPoint and Excel. */
export function isPreviewableOfficePath(path: string): boolean {
  return /\.(doc|docx|ppt|pptx|xls|xlsx)$/i.test(path);
}

/** Rejects with the signal's reason once `signal` has aborted. */
function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted) throw signal.reason ?? new DOMException("The read was abandoned.", "AbortError");
}

/**
 * Reads the whole of a text file of `size` bytes, a range at a time, into one
 * buffer allocated once from the stat. Ranges are sequential rather than
 * parallel: the disk reads ahead of a sequential scan, and one outstanding IPC
 * reply keeps the webview's memory at one buffer plus one range. An aborted
 * `signal` stops it before the next range, so a file switched away from does
 * not go on reading tens of megabytes nobody will see.
 */
async function readWhole(projectId: string, path: string, size: number, signal?: AbortSignal): Promise<Uint8Array> {
  const buffer = new Uint8Array(size);
  let offset = 0;
  while (offset < size) {
    throwIfAborted(signal);
    const length = Math.min(PROJECT_READ_RANGE_MAX_BYTES, size - offset);
    const chunk = await projectFileReadBytes(projectId, path, offset, length);
    const taken = Math.min(chunk.length, length);
    buffer.set(taken === chunk.length ? chunk : chunk.subarray(0, taken), offset);
    offset += taken;
    // Fewer bytes than asked means the end of the file: it shrank since its
    // stat. The stat taken after the read reports it.
    if (taken < length) break;
  }
  throwIfAborted(signal);
  return offset === size ? buffer : buffer.subarray(0, offset);
}

/** Reads of a file that keeps changing under them before the load gives up. */
const LOAD_ATTEMPTS = 3;

/**
 * Loads `path`: stat first, then the whole text or, past `wholeLimit`, a
 * windowed handle.
 *
 * A whole read is bracketed by two stats. If they differ, the bytes may mix
 * two versions of the file — a shrink mid-read even cuts a character in half
 * — so the read starts over, and the stat the view keeps is one the bytes are
 * known to match.
 */
export async function loadProjectFile(
  projectId: string,
  path: string,
  wholeLimit: number = PROJECT_FILE_LIMITS.wholeLoadBytes,
  signal?: AbortSignal,
): Promise<LoadedFile> {
  for (let attempt = 1; ; attempt += 1) {
    throwIfAborted(signal);
    const stat = await projectFileStat(projectId, path);
    throwIfAborted(signal);
    if (stat.version !== 1) throw new Error("the file response used an unsupported version");
    if (stat.path !== path) throw new Error("the response named a different file");
    if (isOfficePath(path)) {
      if (stat.kind === "sensitive_withheld" || stat.kind === "too_large") {
        return { mode: "opaque", stat };
      }
      let document: ProjectDocumentRead | null = null;
      let failure: unknown = null;
      try {
        const candidate = await projectDocumentRead(projectId, path);
        throwIfAborted(signal);
        if (candidate == null || typeof candidate !== "object" || Array.isArray(candidate)) {
          throw new Error("the document response was invalid");
        }
        if (candidate.version !== 1 || candidate.path !== path) {
          throw new Error("the document response did not match this file");
        }
        if (typeof candidate.text !== "string") {
          throw new Error("the document response had invalid text");
        }
        if (!Number.isSafeInteger(candidate.size_bytes) || candidate.size_bytes < 0) {
          throw new Error("the document response had an invalid size");
        }
        if (candidate.modified_ms !== null && !Number.isSafeInteger(candidate.modified_ms)) {
          throw new Error("the document response had an invalid modified time");
        }
        document = candidate;
      } catch (cause: unknown) {
        throwIfAborted(signal);
        failure = cause;
      }
      const after = await projectFileStat(projectId, path);
      throwIfAborted(signal);
      if (after.path !== path) throw new Error("the response named a different file");
      if (!sameVersion(document ?? stat, after)) {
        if (attempt < LOAD_ATTEMPTS) continue;
        throw new Error("the file kept changing while it was read");
      }
      if (document === null) throw failure;
      if (!sameVersion(stat, document)) {
        if (attempt < LOAD_ATTEMPTS) continue;
        throw new Error("the file kept changing while it was read");
      }
      return { mode: "document", stat: document, text: document.text };
    }
    if (stat.kind !== "text") return { mode: "opaque", stat };
    if (stat.size_bytes > wholeLimit) return { mode: "windowed", stat };
    let bytes: Uint8Array | null = null;
    let failure: unknown = null;
    try {
      bytes = await readWhole(projectId, path, stat.size_bytes, signal);
    } catch (cause: unknown) {
      throwIfAborted(signal);
      // A range past a shrunken end is refused; whether that is the file
      // changing or a real failure, the second stat decides.
      failure = cause;
    }
    const after = await projectFileStat(projectId, path);
    throwIfAborted(signal);
    if (!sameVersion(stat, after)) {
      if (attempt < LOAD_ATTEMPTS) continue;
      throw new Error("the file kept changing while it was read");
    }
    if (bytes === null) throw failure;
    return decodeWhole(stat, bytes);
  }
}

function decodeWhole(stat: TextStat, bytes: Uint8Array): LoadedFile {
  try {
    // Fatal: a byte that is not UTF-8 must not be silently replaced in a file
    // the person might edit and save back. The BOM is kept for the same
    // reason; the rich view looks past it.
    const text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
    return { mode: "whole", stat, text };
  } catch {
    return {
      mode: "opaque",
      stat: {
        kind: "not_text",
        version: 1,
        path: stat.path,
        size_bytes: stat.size_bytes,
        modified_ms: stat.modified_ms,
        reason: "non_utf8_text",
      },
    };
  }
}

/** One window of a large text file, aligned to whole lines. */
export interface TextWindow {
  /** Byte offset of the first line shown. */
  start: number;
  /** Byte offset just past the last line shown. */
  end: number;
  lines: string[];
  /** Some bytes were not UTF-8 and show as U+FFFD. */
  replaced: boolean;
}

/** A UTF-8 continuation byte, `10xxxxxx`: never the first byte of a character. */
function isContinuation(byte: number): boolean {
  return (byte & 0xc0) === 0x80;
}

/** The first character boundary at or after `index`: past at most three
 * continuation bytes, the most a character carries. */
function characterStart(bytes: Uint8Array, index: number): number {
  let at = index;
  while (at < bytes.length && at - index < 3 && isContinuation(bytes[at]!)) at += 1;
  return at;
}

/** `end`, or earlier so that the character the range ends inside is not cut:
 * a window that stops mid-character leaves that character to the next one. */
function characterEnd(bytes: Uint8Array, end: number): number {
  let lead = end - 1;
  while (lead >= 0 && end - lead <= 3 && isContinuation(bytes[lead]!)) lead -= 1;
  if (lead < 0) return end;
  const byte = bytes[lead]!;
  const width = byte < 0x80 ? 1 : byte >= 0xf0 ? 4 : byte >= 0xe0 ? 3 : byte >= 0xc0 ? 2 : 1;
  return lead + width > end ? lead : end;
}

/**
 * The lines of `path` around byte `offset`: from the first line that starts at
 * or after it (or the file's start), through the last line that ends inside
 * the window (or the file's end). The window is read from one byte before
 * `offset`, so a line that starts exactly there is seen to start there, and
 * one byte past a whole window, so a line ending exactly at the window's edge
 * is seen to end. Paging forward from a window's `end`, or back to a window's
 * `start`, therefore drops and repeats no line.
 *
 * A single line longer than the window is shown cut, at character boundaries,
 * rather than not at all. Decoding is lossy here, unlike a whole load: a window
 * is read-only, so a replaced byte is shown and never written back.
 */
export async function readWindow(projectId: string, path: string, size: number, offset: number): Promise<TextWindow> {
  const start = Math.max(0, Math.min(offset, size));
  const readFrom = start > 0 ? start - 1 : 0;
  const bytes = await projectFileReadBytes(
    projectId,
    path,
    readFrom,
    Math.min(PROJECT_FILE_LIMITS.windowBytes + (start - readFrom), PROJECT_READ_RANGE_MAX_BYTES),
  );
  let from = 0;
  if (start > 0) {
    const newline = bytes.indexOf(0x0a);
    // No newline at all: the middle of one long line, shown from here.
    from = newline === -1 ? characterStart(bytes, start - readFrom) : newline + 1;
  }
  const atEnd = readFrom + bytes.length >= size;
  let to = bytes.length;
  if (!atEnd) {
    const newline = bytes.lastIndexOf(0x0a);
    to = newline >= from ? newline + 1 : characterEnd(bytes, bytes.length);
  }
  if (to <= from && bytes.length > start - readFrom) {
    // No whole line fits: the window sits inside one line longer than itself,
    // the line ending at the window's last byte. Show that line cut from here.
    from = characterStart(bytes, start - readFrom);
    to = atEnd ? bytes.length : characterEnd(bytes, bytes.length);
  }
  const slice = bytes.subarray(from, to);
  // A byte-order mark is one only at the file's first byte.
  const ignoreBOM = readFrom + from > 0;
  let text: string;
  let replaced = false;
  try {
    text = new TextDecoder("utf-8", { fatal: true, ignoreBOM }).decode(slice);
  } catch {
    text = new TextDecoder("utf-8", { fatal: false, ignoreBOM }).decode(slice);
    replaced = true;
  }
  return { start: readFrom + from, end: readFrom + to, lines: splitLines(text), replaced };
}
