use tauri::{Emitter, Runtime};

pub use xfalcon_contracts::chat::ChatProjectionEvent;
pub use xfalcon_contracts::tools::ToolInvocationEvent;
pub use xfalcon_contracts::RunEvent;

pub const RUNTIME_EVENT_CHANNEL: &str = "xfalcon://runtime-event";
pub const CHAT_RUN_EVENT_CHANNEL: &str = "xfalcon://chat-run-event";
pub const CHAT_PROJECTION_EVENT_CHANNEL: &str = "xfalcon://chat-projection-event";
pub const CHAT_INFERENCE_EVENT_CHANNEL: &str = "xfalcon://chat-inference-event";
/// Live tool-invocation lifecycle (requested/approval-required/approved/
/// denied/started/progress/completed/failed/cancelled) and, by extension, a
/// run's approval-pause state — the frontend observability surface, mirroring
/// the existing `emit_chat_*` channels' shape exactly.
///
/// Emitted by `crate::chat::invocation_emit::EmittingInvocationJournal`, which
/// records into the real journal first and emits only if that succeeded: the
/// UI never shows a step the journal rejected.
pub const TOOL_INVOCATION_EVENT_CHANNEL: &str = "xfalcon://tool-invocation-event";

pub fn emit_runtime_event<R: Runtime>(
    app: &tauri::AppHandle<R>,
    event: &xfalcon_contracts::RuntimeEvent,
) -> Result<(), tauri::Error> {
    app.emit(RUNTIME_EVENT_CHANNEL, event)
}

pub fn emit_chat_run_event<R: Runtime>(
    app: &tauri::AppHandle<R>,
    event: &RunEvent,
) -> Result<(), tauri::Error> {
    app.emit(CHAT_RUN_EVENT_CHANNEL, event)
}

pub fn emit_chat_inference_event<R: Runtime>(
    app: &tauri::AppHandle<R>,
    event: &xfalcon_contracts::inference::InferenceEvent,
) -> Result<(), tauri::Error> {
    app.emit(CHAT_INFERENCE_EVENT_CHANNEL, event)
}

pub fn emit_chat_projection_event<R: Runtime>(
    app: &tauri::AppHandle<R>,
    event: &ChatProjectionEvent,
) -> Result<(), tauri::Error> {
    app.emit(CHAT_PROJECTION_EVENT_CHANNEL, event)
}

pub fn emit_tool_invocation_event<R: Runtime>(
    app: &tauri::AppHandle<R>,
    event: &ToolInvocationEvent,
) -> Result<(), tauri::Error> {
    app.emit(TOOL_INVOCATION_EVENT_CHANNEL, event)
}
