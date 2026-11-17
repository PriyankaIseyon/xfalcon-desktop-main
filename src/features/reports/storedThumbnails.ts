import { previewDocumentSrc, reportOpen, reportThumbnail } from "../../lib/native";

/** How many decoded thumbnails stay in memory. One is at most 200 KiB and most
 * are a few tens, so this is a few megabytes, and it covers a library scrolled
 * end to end without asking DA again. */
const MAX_CACHED = 160;

/** Object URLs by `report id : thumbnail version`, oldest first. A promise, so
 * two cards for one report (grid and pinned) share one request. */
const cache = new Map<string, Promise<string>>();
/** The same URLs once they have arrived, so a card can paint one at once. */
const ready = new Map<string, string>();

/** Raised when DA holds no thumbnail for the report: the card draws the report
 * itself instead. */
export class ThumbnailMissing extends Error {}

/**
 * The object URL of a report's stored thumbnail, asked of DA once per version.
 *
 * `version` is the report's `thumbnail_updated_at_ms`: a new version is a new
 * image, so it is a new key, and the old one is let go at once.
 */
export function storedThumbnailUrl(reportId: string, version: bigint): Promise<string> {
  const key = `${reportId}:${version}`;
  const hit = cache.get(key);
  if (hit !== undefined) {
    // Most recently used goes last, so eviction takes the oldest.
    cache.delete(key);
    cache.set(key, hit);
    return hit;
  }
  for (const stale of [...cache.keys()].filter((other) => other.startsWith(`${reportId}:`))) {
    release(stale);
  }
  const pending = reportThumbnail(reportId).then(
    (image) => {
      if (image.id !== reportId) throw new Error("thumbnail answered for another report");
      const binary = atob(image.data_base64);
      const bytes = new Uint8Array(binary.length);
      for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
      const url = URL.createObjectURL(new Blob([bytes], { type: `image/${image.format}` }));
      if (cache.get(key) === pending) ready.set(key, url);
      return url;
    },
    (cause: unknown) => {
      const code = typeof cause === "object" && cause !== null && "code" in cause ? cause.code : null;
      throw code === "not_found" ? new ThumbnailMissing() : cause;
    },
  );
  cache.set(key, pending);
  // A failure is not kept: the next card that shows the report asks again.
  pending.catch(() => {
    if (cache.get(key) === pending) cache.delete(key);
  });
  while (cache.size > MAX_CACHED) {
    const oldest = cache.keys().next().value;
    if (oldest === undefined) break;
    release(oldest);
  }
  return pending;
}

/** The URL of an image already in memory, or `null` when it would have to be asked for. */
export function peekStoredThumbnailUrl(reportId: string, version: bigint): string | null {
  return ready.get(`${reportId}:${version}`) ?? null;
}

function release(key: string) {
  const pending = cache.get(key);
  cache.delete(key);
  ready.delete(key);
  void pending?.then(URL.revokeObjectURL, () => undefined);
}

/** How many opened card documents stay known. Each is an id into the local
 * preview store, so this holds no page content, only skips asking DA again. */
const MAX_DOCUMENTS = 48;

/** Preview-store URLs of reports drawn live on a card, by `report id : updated
 * at`, oldest first. A card that comes back (a search cleared, a layout
 * switched) draws from the same document instead of asking DA to open the
 * report again. */
const documents = new Map<string, Promise<string>>();
const documentsReady = new Map<string, string>();

/** The preview-store URL a card draws a report without a stored image from,
 * asked of DA once per saved version. */
export function thumbnailDocumentSrc(reportId: string, updatedAt: bigint): Promise<string> {
  const key = `${reportId}:${updatedAt}`;
  const hit = documents.get(key);
  if (hit !== undefined) {
    documents.delete(key);
    documents.set(key, hit);
    return hit;
  }
  for (const stale of [...documents.keys()].filter((other) => other.startsWith(`${reportId}:`))) {
    forgetDocument(stale);
  }
  const pending = reportOpen(reportId).then((document) => {
    if (document.id !== reportId) throw new Error("document answered for another report");
    const src = previewDocumentSrc(document.document_id);
    if (documents.get(key) === pending) documentsReady.set(key, src);
    return src;
  });
  documents.set(key, pending);
  pending.catch(() => {
    if (documents.get(key) === pending) forgetDocument(key);
  });
  while (documents.size > MAX_DOCUMENTS) {
    const oldest = documents.keys().next().value;
    if (oldest === undefined) break;
    forgetDocument(oldest);
  }
  return pending;
}

/** The URL of a card document already opened, or `null` when DA would be asked. */
export function peekThumbnailDocumentSrc(reportId: string, updatedAt: bigint): string | null {
  return documentsReady.get(`${reportId}:${updatedAt}`) ?? null;
}

function forgetDocument(key: string) {
  documents.delete(key);
  documentsReady.delete(key);
}

/** Lets every thumbnail go, on sign-out: none outlives the account it was read for. */
export function clearStoredThumbnails() {
  for (const key of [...cache.keys()]) release(key);
  for (const key of [...documents.keys()]) forgetDocument(key);
}
