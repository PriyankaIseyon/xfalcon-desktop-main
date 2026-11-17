export const MIGRATION_ACK_TARGET = "/index.html?xfalcon-storage-migration=v1";

type ClearableStorage = Pick<Storage, "clear" | "length">;

export function runStorageMigration(
  local: ClearableStorage,
  session: ClearableStorage,
  navigate: (target: string) => void,
): void {
  local.clear();
  session.clear();
  if (local.length !== 0 || session.length !== 0) {
    throw new Error("origin storage migration incomplete");
  }
  navigate(MIGRATION_ACK_TARGET);
}

if (typeof window !== "undefined") {
  try {
    runStorageMigration(window.localStorage, window.sessionStorage, (target) => window.location.replace(target));
  } catch {
    // Fail closed: native startup timeout exits while this window remains hidden.
  }
}
