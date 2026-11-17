/**
 * User-facing recovery copy — keep aligned with
 * `agent::recovery::recover_interrupted_run_policy` in the Tauri crate.
 */
export const RECOVERY_NOTICE_INTERRUPTED =
  "This response was interrupted before it finished. You can retry when ready." as const;

export const RECOVERY_NOTICE_RECONCILE =
  "A previous response may still be syncing with the server. Open this conversation to reconcile before retrying." as const;

export function recoveryNoticeForRunState(state: string): string | null {
  if (state === "interrupted" || state === "failed") {
    return RECOVERY_NOTICE_INTERRUPTED;
  }
  return null;
}
