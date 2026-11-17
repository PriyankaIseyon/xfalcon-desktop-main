use tauri::Manager;

use crate::app::commands::AppRuntimeState;

#[tauri::command]
pub async fn workspace_status(
    app: tauri::AppHandle,
) -> Result<xfalcon_contracts::workspace::WorkspaceStatus, String> {
    app.state::<AppRuntimeState>()
        .workspace()
        .status()
        .await
        .map_err(|error| {
            tracing::warn!(%error, "workspace status failed");
            error.to_string()
        })
}
