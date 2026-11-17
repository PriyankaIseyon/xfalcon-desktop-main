import { useSyncExternalStore } from "react";
import { getAuthSession, subscribeAuthSession } from "./authStore";

export function useAuthSession() {
  return useSyncExternalStore(subscribeAuthSession, getAuthSession, getAuthSession);
}

export function isAuthenticated(session: ReturnType<typeof getAuthSession>): boolean {
  return session.status === "authenticated";
}
