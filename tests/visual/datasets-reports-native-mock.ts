import type { Page } from "@playwright/test";
import type {
  ChatModelCatalogView,
  ChatConversationSummary,
  ChatConversationView,
  DatasetDetail,
  DatasetListResult,
  DatasetPreview,
  DatasetUploadStartResult,
  DocumentExportResult,
  ReportDocument,
  ReportListResult,
  ReportView,
  StagedDatasetView,
  UploadQueueV1,
} from "../../src/lib/generated/contracts";

export type NativeOutcome<T> = { kind: "ok"; value: T } | { kind: "error"; value: unknown } | { kind: "pending" };

export const ok = <T>(value: T): NativeOutcome<T> => ({ kind: "ok", value });
export const fail = <T = never>(value: unknown): NativeOutcome<T> => ({ kind: "error", value });
export const pending = <T = never>(): NativeOutcome<T> => ({ kind: "pending" });

export type DatasetReportsMockConfig = {
  theme?: "light" | "dark";
  modelCatalog?: ChatModelCatalogView;
  datasets?: NativeOutcome<DatasetListResult>[];
  datasetDetails?: NativeOutcome<DatasetDetail>[];
  datasetPick?: NativeOutcome<StagedDatasetView>[];
  datasetPreviews?: NativeOutcome<DatasetPreview>[];
  datasetUploadStart?: NativeOutcome<DatasetUploadStartResult>[];
  datasetDelete?: NativeOutcome<void>[];
  datasetDownload?: NativeOutcome<DocumentExportResult>[];
  uploadQueue?: UploadQueueV1;
  uploadRetry?: NativeOutcome<UploadQueueV1>[];
  uploadCancel?: NativeOutcome<UploadQueueV1>[];
  reports?: NativeOutcome<ReportListResult>[];
  reportOpen?: NativeOutcome<ReportDocument>[];
  reportRename?: NativeOutcome<ReportView>[];
  reportSetPinned?: NativeOutcome<ReportView>[];
  reportDelete?: NativeOutcome<void>[];
  chatSummary?: ChatConversationSummary;
  chatConversation?: ChatConversationView;
};

const bootstrap = {
  version: 1,
  application_version: "0.1.0-visual-test",
  platform: "windows",
  session: {
    status: "authenticated",
    version: 1,
    identity: {
      version: 1,
      user_id: "visual-user",
      tenant_id: "visual-tenant",
      display_name: "Visual Test",
      email: null,
    },
    expires_at_ms: Date.now() + 3_600_000,
  },
} as const;

const EMPTY_UPLOADS: UploadQueueV1 = { version: 1, revision: 0, uploads: [] };
const EMPTY_DATASETS: DatasetListResult = {
  version: 1,
  datasets: [],
  uploads_enabled: true,
  truncated: false,
};
const EMPTY_REPORTS: ReportListResult = { version: 1, reports: [], project_origins: [] };

export async function installDatasetReportsMock(page: Page, config: DatasetReportsMockConfig = {}): Promise<void> {
  await page.addInitScript(
    ({ bootstrapValue, cfg, emptyDatasets, emptyReports, emptyUploads }) => {
      type Outcome = { kind: "ok"; value: unknown } | { kind: "error"; value: unknown } | { kind: "pending" };
      type Waiter = { resolve: (value: unknown) => void; reject: (reason: unknown) => void };

      const queues: Record<string, Outcome[]> = {
        dataset_list: [...(cfg.datasets ?? [{ kind: "ok", value: emptyDatasets }])],
        dataset_detail: [...(cfg.datasetDetails ?? [])],
        dataset_pick: [...(cfg.datasetPick ?? [])],
        dataset_preview: [...(cfg.datasetPreviews ?? [])],
        dataset_upload_start: [...(cfg.datasetUploadStart ?? [])],
        dataset_delete: [...(cfg.datasetDelete ?? [])],
        dataset_download: [...(cfg.datasetDownload ?? [])],
        upload_retry: [...(cfg.uploadRetry ?? [])],
        upload_cancel: [...(cfg.uploadCancel ?? [])],
        report_list: [...(cfg.reports ?? [{ kind: "ok", value: emptyReports }])],
        report_open: [...(cfg.reportOpen ?? [])],
        report_rename: [...(cfg.reportRename ?? [])],
        report_set_pinned: [...(cfg.reportSetPinned ?? [])],
        report_delete: [...(cfg.reportDelete ?? [])],
      };
      const coalesced = new Map<string, Waiter[]>();
      const callbacks = new Map<number, (data: unknown) => void>();
      const listeners = new Map<string, number[]>();
      let nextCallback = 1;

      const nextFrom = (command: string): Promise<unknown> =>
        new Promise((resolve, reject) => {
          const current = coalesced.get(command);
          if (current !== undefined) {
            current.push({ resolve, reject });
            return;
          }
          const waiters: Waiter[] = [{ resolve, reject }];
          coalesced.set(command, waiters);
          setTimeout(() => {
            coalesced.delete(command);
            const queue = queues[command] ?? [];
            const outcome = queue.length > 1 ? queue.shift() : queue[0];
            for (const waiter of waiters) {
              if (outcome === undefined) {
                waiter.reject(new Error(`No visual outcome for ${command}`));
              } else if (outcome.kind === "ok") {
                waiter.resolve(outcome.value);
              } else if (outcome.kind === "error") {
                waiter.reject(outcome.value);
              }
            }
          }, 0);
        });

      const transformCallback = (callback: (data: unknown) => void, once = false) => {
        const id = nextCallback++;
        callbacks.set(id, (data) => {
          callback(data);
          if (once) callbacks.delete(id);
        });
        return id;
      };
      const emit = (event: string, payload: unknown) => {
        for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, payload });
      };

      Object.assign(window, {
        __DATASET_REPORT_EMIT_UPLOAD__: (queue: unknown) => emit("xfalcon://upload-queue", queue),
      });

      Object.defineProperty(window, "__TAURI_INTERNALS__", {
        configurable: false,
        value: Object.freeze({
          transformCallback,
          convertFileSrc(documentId: string) {
            return `http://xf-preview.localhost/${documentId}`;
          },
          invoke(command: string, args?: Record<string, unknown>) {
            if (command === "plugin:event|listen") {
              const event = String(args?.event);
              const handler = Number(args?.handler);
              listeners.set(event, [...(listeners.get(event) ?? []), handler]);
              return Promise.resolve(handler);
            }
            if (command === "plugin:event|unlisten") {
              const event = String(args?.event);
              const eventId = Number(args?.eventId);
              listeners.set(
                event,
                (listeners.get(event) ?? []).filter((id) => id !== eventId),
              );
              return Promise.resolve(null);
            }
            if (command === "app_bootstrap") return Promise.resolve(bootstrapValue);
            if (command === "app_health") return Promise.resolve({ version: 1, status: "ready" });
            if (command === "settings_appearance_get") {
              return Promise.resolve({
                version: 1,
                theme: cfg.theme ?? "light",
                reading_size: "small",
                show_reasoning: false,
                show_follow_ups: true,
              });
            }
            if (command === "chat_model_catalog" && cfg.modelCatalog) return Promise.resolve(cfg.modelCatalog);
            if (command === "workspace_status") {
              return Promise.resolve({
                version: 1,
                tenant_host: "tenant.example",
                mcp_resource_url: "https://tenant.example/mcp",
                mcp_servers: [],
                models_available: 1,
                default_model_id: "gpt-test",
                default_provider: "openai",
                chat_ready: true,
                setup_note: "Ready",
              });
            }
            if (command === "chat_workspace_bootstrap") {
              return Promise.resolve({
                version: 1,
                conversations: cfg.chatSummary === undefined ? [] : [cfg.chatSummary],
                recovery_notice: null,
              });
            }
            if (command === "chat_conversation_open" && cfg.chatConversation !== undefined) {
              return Promise.resolve(cfg.chatConversation);
            }
            if (command === "preview_document_register") return Promise.resolve("chat-preview-document");
            if (command === "project_list") return Promise.resolve({ version: 1, projects: [] });
            if (command === "diagnostics_log_renderer_error") return Promise.resolve(undefined);
            if (command === "upload_queue") return Promise.resolve(cfg.uploadQueue ?? emptyUploads);
            if (command === "dataset_stage_discard") return Promise.resolve(undefined);
            if (command === "report_open" && cfg.reportOpen === undefined) {
              return Promise.resolve({
                version: 1,
                id: String(args?.reportId),
                title: "Saved report",
                description: null,
                document_id: `thumbnail-${String(args?.reportId)}`,
              });
            }
            if (command in queues) return nextFrom(command);
            return Promise.reject(new Error(`Visual test rejected unknown native command: ${command}`));
          },
        }),
        writable: false,
      });
    },
    {
      bootstrapValue: bootstrap,
      cfg: config,
      emptyDatasets: EMPTY_DATASETS,
      emptyReports: EMPTY_REPORTS,
      emptyUploads: EMPTY_UPLOADS,
    },
  );
}

declare global {
  interface Window {
    __DATASET_REPORT_EMIT_UPLOAD__?: (queue: UploadQueueV1) => void;
  }
}
