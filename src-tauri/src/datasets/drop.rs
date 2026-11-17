//! Dropping a file on the window.
//!
//! Handled here, in the backend, from `WindowEvent::DragDrop`. The webview
//! never receives the dropped paths.
//!
//! Skill drops follow the same rule through a different route: they are
//! granted in Rust (`crate::skills::drop`) and the renderer imports by the
//! grant id, never by the dropped paths. A dataset drop reaches tenant storage,
//! so the rule the picker already follows — the only way a path enters the
//! system is a person acting on an OS surface — has to hold for the drop too.
//!
//! The renderer is told the *outcome*, and decides whether to show it: a drop
//! that lands while the person is reading a conversation is staged all the same
//! but has nothing to render. That is a display decision. Nothing the webview
//! believes about which destination is active can change what a drop is allowed
//! to do, because the webview is never asked.

use tauri::{Emitter as _, Manager as _, WindowEvent};
use xfalcon_contracts::datasets::{DatasetDropOutcome, DatasetErrorCode, DatasetOperationError};

use crate::datasets::commands;

/// Event the datasets surface listens on.
pub const DROP_EVENT: &str = "xfalcon://dataset-dropped";

/// Files accepted from one drop.
///
/// A person dropping a folder full of exports means the first few, not a
/// hundred staged entries — and the staging cap would refuse the rest anyway,
/// which reads as an error rather than as a bound. Kept at the staging cap so
/// one drop can fill it exactly once.
const MAX_DROPPED_FILES: usize = crate::datasets::staging::MAX_STAGED_DATASETS;

/// Watches the main window for drops and stages what lands on it.
pub fn register_drop_handler(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        // No window means nothing to drop onto. Not an error: the caller is
        // running headless, which the test harness does.
        return;
    };
    let handle = app.clone();
    window.on_window_event(move |event| {
        let WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        let handle = handle.clone();
        let paths = paths.clone();
        let account_generation = handle
            .state::<std::sync::Arc<crate::datasets::Staging>>()
            .current_account_generation();
        tauri::async_runtime::spawn(async move {
            stage_dropped(&handle, paths, account_generation).await;
        });
    });
}

/// Stages each dropped path, reporting every outcome separately.
///
/// One event per file rather than one for the batch: a person who drops a
/// spreadsheet and a PDF together needs to see which one was refused.
async fn stage_dropped(
    app: &tauri::AppHandle,
    paths: Vec<std::path::PathBuf>,
    account_generation: u64,
) {
    let past_the_cap = paths.len() > MAX_DROPPED_FILES;
    for path in paths.into_iter().take(MAX_DROPPED_FILES) {
        let outcome = match commands::stage(app, path, account_generation).await {
            Ok(staged) => DatasetDropOutcome::staged(staged),
            Err(error) => DatasetDropOutcome::refused(error),
        };
        report(app, &outcome);
    }
    if past_the_cap {
        // Said once for the drop, not once per file: a person who dropped a
        // folder does not need eight refusals to learn the bound — and the
        // files past it being taken silently would read as them uploading.
        report(
            app,
            &DatasetDropOutcome::refused(DatasetOperationError::new(
                DatasetErrorCode::StageLimitReached,
            )),
        );
    }
}

fn report(app: &tauri::AppHandle, outcome: &DatasetDropOutcome) {
    if let Err(error) = app.emit(DROP_EVENT, outcome) {
        tracing::warn!(%error, "dataset drop outcome not delivered");
    }
}
