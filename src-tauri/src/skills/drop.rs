//! Dropping skill files or folders on the window.
//!
//! Handled here, from `WindowEvent::DragDrop`, so the webview never receives a
//! dropped path. The drop is recorded as a skill-source grant
//! (`crate::path_grants`) and the webview is told only that the grant exists:
//! it imports by the grant id, as it does after a picker.
//!
//! Enter and over are reported as one signal, the same one a drop target
//! shows. Leave clears it. Only a drop that carries paths is a grant.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{Emitter as _, Manager as _, WindowEvent};

use super::commands::grant_sources;
use crate::path_grants::PathGrants;

/// Something is being carried over the window (`"over"`) or has left it
/// (`"leave"`). Payload is the bare string.
pub const DRAG_EVENT: &str = "xfalcon://skill-drag";

/// A drop that carries sources. Payload is a [`SkillSourceGrantView`].
pub const DROPPED_EVENT: &str = "xfalcon://skill-sources-dropped";

/// A drop that carries sources but cannot be granted, such as one over the
/// grant limit. Payload is a [`SkillOperationError`] naming why.
pub const REJECTED_EVENT: &str = "xfalcon://skill-sources-rejected";

/// What one window drag event means to the skills import surface.
#[derive(Debug, PartialEq, Eq)]
enum DragSignal {
    Over,
    Leave,
    Dropped(Vec<PathBuf>),
}

/// Watches the main window for skill drops. Registered alongside the dataset
/// drop handler, which keeps its own copy of the event.
pub fn register_skill_drop_handler(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        // No window means nothing to drop onto. Not an error: the caller is
        // running headless, which the test harness does.
        return;
    };
    let handle = app.clone();
    window.on_window_event(move |event| {
        let WindowEvent::DragDrop(drag) = event else {
            return;
        };
        match signal(drag) {
            Some(DragSignal::Over) => emit(&handle, DRAG_EVENT, "over"),
            Some(DragSignal::Leave) => emit(&handle, DRAG_EVENT, "leave"),
            Some(DragSignal::Dropped(paths)) => {
                let grants = handle.state::<Arc<PathGrants>>();
                match grant_sources(&grants, paths) {
                    Ok(Some(view)) => emit(&handle, DROPPED_EVENT, &view),
                    Ok(None) => {}
                    Err(error) => emit(&handle, REJECTED_EVENT, error),
                }
            }
            None => {}
        }
    });
}

fn signal(event: &tauri::DragDropEvent) -> Option<DragSignal> {
    match event {
        tauri::DragDropEvent::Enter { .. } | tauri::DragDropEvent::Over { .. } => {
            Some(DragSignal::Over)
        }
        tauri::DragDropEvent::Leave => Some(DragSignal::Leave),
        tauri::DragDropEvent::Drop { paths, .. } if !paths.is_empty() => {
            Some(DragSignal::Dropped(paths.clone()))
        }
        // A drop with nothing in it is not a choice.
        tauri::DragDropEvent::Drop { .. } => None,
        _ => None,
    }
}

fn emit<P: serde::Serialize + Clone>(app: &tauri::AppHandle, event: &str, payload: P) {
    if let Err(error) = app.emit(event, payload) {
        tracing::warn!(%error, "skill drop signal not delivered");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::PhysicalPosition;

    fn position() -> PhysicalPosition<f64> {
        PhysicalPosition::new(0.0, 0.0)
    }

    #[test]
    fn enter_and_over_are_one_signal_and_leave_is_the_other() {
        assert_eq!(
            signal(&tauri::DragDropEvent::Enter {
                paths: vec![PathBuf::from("/chosen/a.md")],
                position: position(),
            }),
            Some(DragSignal::Over)
        );
        assert_eq!(
            signal(&tauri::DragDropEvent::Over {
                position: position(),
            }),
            Some(DragSignal::Over)
        );
        assert_eq!(
            signal(&tauri::DragDropEvent::Leave),
            Some(DragSignal::Leave)
        );
    }

    #[test]
    fn a_drop_with_paths_carries_them_and_an_empty_drop_carries_nothing() {
        let paths = vec![PathBuf::from("/chosen/a.md"), PathBuf::from("/chosen/lib")];
        assert_eq!(
            signal(&tauri::DragDropEvent::Drop {
                paths: paths.clone(),
                position: position(),
            }),
            Some(DragSignal::Dropped(paths))
        );
        assert_eq!(
            signal(&tauri::DragDropEvent::Drop {
                paths: Vec::new(),
                position: position(),
            }),
            None
        );
    }

    #[test]
    fn a_dropped_selection_becomes_a_view_with_its_count_and_first_names() {
        let grants = PathGrants::default();
        let paths: Vec<PathBuf> = (0..8)
            .map(|index| PathBuf::from(format!("/private/folder/skill-{index}.zip")))
            .collect();

        let view = grant_sources(&grants, paths)
            .expect("a drop within the limit is not refused")
            .expect("a drop is granted");

        assert_eq!(view.count, 8);
        assert_eq!(view.labels.len(), 5);
        assert_eq!(view.labels[0], "skill-0.zip");
    }

    #[test]
    fn the_dropped_view_carries_no_path_the_renderer_could_act_on() {
        let grants = PathGrants::default();
        let view = grant_sources(
            &grants,
            vec![PathBuf::from("/private/folder/bad-skill.zip")],
        )
        .expect("a drop within the limit is not refused")
        .expect("a drop is granted");

        let serialized = serde_json::to_string(&view).expect("view serializes");
        assert!(!serialized.contains("/private"));
        assert!(!serialized.contains("folder"));
        assert!(serialized.contains("bad-skill.zip"));
        // The grant is what the renderer imports with, and it resolves here.
        assert!(grants
            .resolve(crate::path_grants::GrantKind::SkillSources, &view.grant_id)
            .is_some());
    }
}
