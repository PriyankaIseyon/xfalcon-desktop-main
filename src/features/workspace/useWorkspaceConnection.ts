import { useCallback, useEffect, useState } from "react";
import { workspaceStatus } from "../../lib/native";
import type { WorkspaceStatus } from "../../lib/generated/contracts";

export function useWorkspaceConnection() {
  const [status, setStatus] = useState<WorkspaceStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setStatus(await workspaceStatus());
    } catch (cause) {
      setStatus(null);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { status, error, loading, refresh };
}
