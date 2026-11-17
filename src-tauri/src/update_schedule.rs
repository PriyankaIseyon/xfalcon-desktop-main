use std::time::Duration;

use tauri::{AppHandle, Emitter, Runtime};

pub const UPDATE_CHECK_EVENT: &str = "xfalcon://update-check-requested";
pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);

/// Owns the periodic update cadence outside the renderer privilege boundary.
pub fn spawn<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(UPDATE_CHECK_INTERVAL).await;
            if let Err(error) = app.emit(UPDATE_CHECK_EVENT, ()) {
                tracing::warn!(error = %error, "update: failed to request scheduled check");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cadence_is_four_hours() {
        assert_eq!(UPDATE_CHECK_INTERVAL, Duration::from_secs(14_400));
        assert_eq!(UPDATE_CHECK_EVENT, "xfalcon://update-check-requested");
    }
}
