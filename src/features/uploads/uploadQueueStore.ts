import { useSyncExternalStore } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type { UploadProjectionV1, UploadQueueV1 } from "../../lib/generated/contracts";
import { onUploadQueue, uploadQueue } from "../../lib/native";

/**
 * The upload queue, in its own store.
 *
 * Progress events would otherwise sit in `chatStore` and wake the transcript
 * on every byte. Draft typing already proved that pattern is too expensive
 * (spec: chat-streaming-performance).
 *
 * One store, not one per surface: there is one queue in the backend and one
 * `xfalcon://upload-queue` event, so a second bridge would listen twice and the
 * two would disagree the moment either missed an event. The composer dock reads
 * the rows bound to its conversation, the Datasets destination reads the
 * persistent-dataset rows, and both filter the same array.
 */

const EMPTY_QUEUE: UploadQueueV1 = { version: 1, revision: 0, uploads: [] };

let queue: UploadQueueV1 = EMPTY_QUEUE;
const listeners = new Set<() => void>();
let started = false;
let unlisten: UnlistenFn | null = null;

function emit() {
  for (const listener of listeners) listener();
}

export function getUploadQueue(): UploadQueueV1 {
  return queue;
}

export function applyUploadQueue(next: UploadQueueV1): void {
  if (next.revision <= queue.revision) return;
  queue = next;
  emit();
}

export function resetUploadQueue(): void {
  // Keep the native revision fence. A delayed event from the account that was
  // just left must not repopulate the queue after the account stores reset.
  queue = { ...EMPTY_QUEUE, revision: queue.revision };
  emit();
}

export function resetUploadQueueForTests(): void {
  queue = EMPTY_QUEUE;
  listeners.clear();
}

export function subscribeUploadQueue(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useUploadQueue(): UploadProjectionV1[] {
  return useSyncExternalStore(
    subscribeUploadQueue,
    () => getUploadQueue().uploads,
    () => EMPTY_QUEUE.uploads,
  );
}

export async function startUploadQueueBridge(): Promise<void> {
  if (started) return;
  if (!(globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__) {
    return;
  }
  started = true;
  try {
    applyUploadQueue(await uploadQueue());
    unlisten = await onUploadQueue((next) => {
      applyUploadQueue(next);
    });
  } catch {
    started = false;
    unlisten = null;
  }
}

export async function stopUploadQueueBridge(): Promise<void> {
  if (unlisten) {
    await unlisten();
    unlisten = null;
  }
  started = false;
}
