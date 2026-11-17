import type { SessionProjection } from "../../lib/generated/contracts";
import { resetCanvasStore } from "../canvas/canvasStore";
import { resetChatStore } from "../chat/chatStore";
import { resetDatasetsStore } from "../datasets/datasetsStore";
import { resetPreviewCaptureAttempts } from "../reports/reportPreviewCapture";
import { resetReportsStore } from "../reports/reportsStore";
import { clearSignedOutNotice } from "./signedOutNotice";

type Listener = () => void;

let session: SessionProjection = { status: "unauthenticated", version: 1 };
let accountGeneration = 0;
const listeners = new Set<Listener>();

export function getAuthSession(): SessionProjection {
  return session;
}

export function setAuthSession(next: SessionProjection): void {
  if (accountKey(session) !== accountKey(next)) {
    accountGeneration += 1;
    resetAccountScopedStores();
  }
  session = next;
  if (next.status === "authenticated") {
    clearSignedOutNotice();
  }
  for (const listener of listeners) {
    listener();
  }
}

/**
 * A token for asynchronous account-scoped work.
 *
 * Callers capture it before an IPC request and compare it after every await.
 * A different value means the result belongs to an account that has already
 * left and must not write into the current account's module stores.
 */
export function getAuthAccountGeneration(): number {
  return accountGeneration;
}

/**
 * Every module store that holds a signed-in account's in-memory view.
 *
 * `AppShell` unmounts on sign-out (`AuthGate` renders `LoginView` in its
 * place), but these are plain module singletons outside React's tree, so
 * unmounting the shell does not clear them — the next account's shell would
 * otherwise mount straight onto whatever the previous account left behind
 * and render it for the render or two before its own reads land: the prior
 * account's open transcript and conversation list (`chatStore`, plus the
 * composer draft, upload queue, and in-progress edit it owns), dataset
 * catalog (`datasetsStore`), and open canvas tabs (`canvasStore`). Reports
 * already had this guarantee (`resetReportsStore`); the rest did not.
 */
function resetAccountScopedStores(): void {
  resetReportsStore();
  resetPreviewCaptureAttempts();
  resetChatStore();
  resetDatasetsStore();
  resetCanvasStore();
}

function accountKey(value: SessionProjection): string | null {
  return value.status === "authenticated" ? `${value.identity.tenant_id}:${value.identity.user_id}` : null;
}

export function subscribeAuthSession(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function resetAuthSessionForTests(): void {
  accountGeneration += 1;
  session = { status: "unauthenticated", version: 1 };
  clearSignedOutNotice();
  listeners.clear();
  resetAccountScopedStores();
}
