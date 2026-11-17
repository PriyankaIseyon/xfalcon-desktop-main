import { memo, useEffect, useState } from "react";
import { Composer, type ComposerProps } from "./Composer";
import { setComposerDraft, useComposerDraft } from "./composerStore";
import { useUploadQueue } from "../uploads/uploadQueueStore";
import { useModelCatalog } from "../models/useModelCatalog";
import { CHATGPT_PLAN_COPY, CLAUDE_RUNTIME_COPY } from "../settings/settingsCopy";
import type { ProviderId } from "../../lib/generated/contracts";
import { mcpServerList } from "../../lib/native";
import { getAuthAccountGeneration } from "../auth/authStore";

/** The plan a selected model runs on, named beside the model. A model on an API key names none. */
const PLAN_NOTE: Partial<Record<ProviderId, string>> = {
  chatgpt_plan: CHATGPT_PLAN_COPY.usingPlan,
  claude_code: CLAUDE_RUNTIME_COPY.usingAccount,
};

export type ComposerDockProps = Omit<ComposerProps, "draft" | "onDraftChange" | "attachments"> & {
  conversationId?: string | null;
  /**
   * Whose draft this composer shows. Defaults to `conversationId`, but the
   * new-chat screen passes `null` even while a reserved conversation holds its
   * files: those files do not make it another chat, and the text typed there
   * must not vanish when one is attached.
   */
  draftOwner?: string | null;
  mcpServerId?: string | null;
  mcpSelectionLoading?: boolean;
  mcpSelectionError?: string | null;
  mcpSelectionDisabled?: boolean;
  onMcpServerSelect?: (serverId: string | null) => void;
};

/**
 * The composer, bound to the draft store.
 *
 * This component exists to be the ONLY subscriber to that store, so a keystroke
 * re-renders the input and nothing above it. `ChatSurface` renders this rather
 * than reading the draft itself, which is what keeps the transcript out of the
 * typing path entirely (spec: chat-streaming-performance). The memo does the
 * same for streamed text: the transcript re-renders per delta, and the composer
 * does not, because `ChatPane` passes its handler props through unchanged. A
 * re-render of `AppShell` itself is different: it creates new inline handler
 * identities (`onSubmit`, `onAttach`, `onStop`, `onRetryRun`, `onCancelUpload`,
 * `onRetryUpload`, `skills`...), so the memo does not skip that render.
 *
 * `Composer` stays a controlled component with no store knowledge: it is
 * rendered on the welcome screen and docked below the transcript, and a
 * component that reached into a module store would be testable in neither
 * position without resetting global state first.
 */
export const ComposerDock = memo(function ComposerDock({
  conversationId = null,
  draftOwner = conversationId,
  mcpServerId = null,
  mcpSelectionLoading = false,
  mcpSelectionError = null,
  mcpSelectionDisabled = false,
  onMcpServerSelect,
  ...props
}: ComposerDockProps) {
  const [serverOptions, setServerOptions] = useState<Array<{ id: string; label: string }>>([]);
  const currentAccountGeneration = getAuthAccountGeneration();
  useEffect(() => {
    let live = true;
    const generation = getAuthAccountGeneration();
    setServerOptions([]);
    void mcpServerList()
      .then((result) => {
        if (!live || generation !== getAuthAccountGeneration()) return;
        setServerOptions(
          result.servers
            .filter((server) => server.enabled && server.unavailable === null && server.permission_mode !== "deny")
            .map((server) => ({ id: server.id, label: server.display_name ?? server.name })),
        );
      })
      .catch(() => {
        if (live && generation === getAuthAccountGeneration()) setServerOptions([]);
      });
    return () => {
      live = false;
    };
  }, [currentAccountGeneration]);
  const draft = useComposerDraft(draftOwner);
  const uploads = useUploadQueue();
  const models = useModelCatalog(conversationId);
  const selectedModel = models.catalog?.models.find((model) => model.id === models.catalog?.selected_model_id);
  const modelName = selectedModel?.display_name;
  // Shown while a plan model is selected, not only during a request, so the plan is visible before sending.
  const modelNote = selectedModel ? PLAN_NOTE[selectedModel.provider] : undefined;
  const attachments = uploads.filter(
    (upload) =>
      upload.purpose === "chat_attachment" && conversationId != null && upload.conversation_id === conversationId,
  );
  const mcpServerLabel = mcpSelectionLoading
    ? "Loading…"
    : mcpSelectionError
      ? "Selection unavailable"
      : mcpServerId === null
        ? "All servers"
        : (serverOptions.find((server) => server.id === mcpServerId)?.label ?? `Unavailable · ${mcpServerId}`);
  return (
    <Composer
      {...props}
      attachments={attachments}
      draft={draft}
      modelName={modelName ?? (models.loading ? "Loading model…" : "Model unavailable")}
      modelNote={modelNote}
      mcpSelection={{
        serverId: mcpServerId,
        loading: mcpSelectionLoading,
        error: mcpSelectionError,
        label: mcpServerLabel,
      }}
      mcpServerOptions={serverOptions}
      mcpSelectionDisabled={mcpSelectionDisabled || mcpSelectionError != null}
      onMcpServerSelect={onMcpServerSelect}
      onDraftChange={(next) => setComposerDraft(next, draftOwner)}
    />
  );
});
