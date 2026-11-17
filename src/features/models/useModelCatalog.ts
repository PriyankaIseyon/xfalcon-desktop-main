import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import type { ChatModelCatalogView } from "../../lib/generated/contracts";
import { getAuthAccountGeneration } from "../auth/authStore";
import { useAuthSession } from "../auth/useAuth";
import { loadModelCatalog } from "./modelNative";

export interface ModelCatalogState {
  catalog: ChatModelCatalogView | null;
  loading: boolean;
  error: string | null;
  refresh: () => void;
}

interface ScopedCatalogState extends Omit<ModelCatalogState, "refresh"> {
  accountGeneration: number;
  conversationId: string | null;
  refreshToken: number;
  invalidationCount: number;
}

type CatalogScope = Omit<ScopedCatalogState, "catalog" | "loading" | "error">;

const catalogInvalidationListeners = new Set<() => void>();
let catalogInvalidationCount = 0;

// The last successful catalog per account and conversation, so a reopened chat shows its model at once
// while the catalog reloads. Once the cap is reached the oldest entry is dropped.
const CATALOG_CACHE_LIMIT = 50;
const catalogCache = new Map<string, ChatModelCatalogView>();

function catalogCacheKey(accountGeneration: number, conversationId: string | null): string {
  return `${accountGeneration}:${conversationId ?? "new"}`;
}

function rememberCatalog(key: string, catalog: ChatModelCatalogView): void {
  catalogCache.delete(key);
  catalogCache.set(key, catalog);
  if (catalogCache.size > CATALOG_CACHE_LIMIT) {
    const [oldestKey] = catalogCache.keys();
    catalogCache.delete(oldestKey);
  }
}

/** The state a scope starts in: its cached catalog when one exists, otherwise a load in progress. */
function stateForScope(scope: CatalogScope): ScopedCatalogState {
  const catalog = catalogCache.get(catalogCacheKey(scope.accountGeneration, scope.conversationId)) ?? null;
  return { ...scope, catalog, loading: catalog === null, error: null };
}

/**
 * The state after the scope changes. An unchanged account and conversation keep the catalog already on screen while
 * a refresh or invalidation reloads it. Anything else starts from the cache.
 */
function nextScopeState(previous: ScopedCatalogState, scope: CatalogScope): ScopedCatalogState {
  if (
    previous.catalog !== null &&
    previous.accountGeneration === scope.accountGeneration &&
    previous.conversationId === scope.conversationId
  ) {
    return { ...scope, catalog: previous.catalog, loading: false, error: null };
  }
  return stateForScope(scope);
}

function getCatalogInvalidationCount(): number {
  return catalogInvalidationCount;
}

function subscribeCatalogInvalidation(listener: () => void): () => void {
  catalogInvalidationListeners.add(listener);
  return () => catalogInvalidationListeners.delete(listener);
}

/** Reloads the model catalog in every mounted picker. Call it after a change that can alter the catalog. */
export function invalidateModelCatalog(): void {
  catalogCache.clear();
  catalogInvalidationCount += 1;
  for (const listener of catalogInvalidationListeners) listener();
}

export function useModelCatalog(conversationId: string | null): ModelCatalogState {
  const invalidationCount = useSyncExternalStore(subscribeCatalogInvalidation, getCatalogInvalidationCount);
  const [state, setState] = useState<ScopedCatalogState>(() =>
    stateForScope({
      accountGeneration: getAuthAccountGeneration(),
      conversationId,
      refreshToken: 0,
      invalidationCount,
    }),
  );
  const [refreshToken, setRefreshToken] = useState(0);
  const session = useAuthSession();
  const accountGeneration = getAuthAccountGeneration();
  const accountKey =
    session.status === "authenticated" ? `${session.identity.tenant_id}:${session.identity.user_id}` : null;

  useEffect(() => {
    let current = true;
    const scope = { accountGeneration, conversationId, refreshToken, invalidationCount };
    const key = catalogCacheKey(accountGeneration, conversationId);
    setState((previous) => nextScopeState(previous, scope));
    void loadModelCatalog(conversationId)
      .then((catalog) => {
        if (current && accountGeneration === getAuthAccountGeneration()) {
          rememberCatalog(key, catalog);
          setState({ ...scope, catalog, loading: false, error: null });
        }
      })
      .catch(() => {
        // A failed reload keeps the catalog on screen. Only a scope with nothing shown reports the error.
        if (current && accountGeneration === getAuthAccountGeneration()) {
          setState((previous) =>
            previous.catalog !== null
              ? previous
              : { ...scope, catalog: null, loading: false, error: "Could not load models." },
          );
        }
      });
    return () => {
      current = false;
    };
  }, [accountKey, accountGeneration, conversationId, refreshToken, invalidationCount]);

  const refresh = useCallback(() => {
    setRefreshToken((token) => token + 1);
  }, []);

  if (
    state.accountGeneration !== accountGeneration ||
    state.conversationId !== conversationId ||
    state.refreshToken !== refreshToken ||
    state.invalidationCount !== invalidationCount
  ) {
    const next = nextScopeState(state, { accountGeneration, conversationId, refreshToken, invalidationCount });
    return { catalog: next.catalog, loading: next.loading, error: next.error, refresh };
  }
  return { catalog: state.catalog, loading: state.loading, error: state.error, refresh };
}
