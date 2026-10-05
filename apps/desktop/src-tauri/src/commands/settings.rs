use std::path::PathBuf;
use serde::Serialize;

#[derive(Serialize)]
pub struct ReceiveFolderInfo {
    pub path: String,
}

fn default_receive_folder() -> PathBuf {
    dirs::download_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("TOVI")
}

/// Returns the current receive folder path.
#[tauri::command]
pub async fn get_receive_folder() -> Result<ReceiveFolderInfo, String> {
    // TODO: read from SQLite settings table via tovi-core storage
    let path = default_receive_folder();
    Ok(ReceiveFolderInfo {
        path: path.to_string_lossy().to_string(),
    })
}

/// Persists a new receive folder preference.
#[tauri::command]
pub async fn set_receive_folder(path: String) -> Result<(), String> {
    // TODO: write to SQLite settings table via tovi-core storage
    tracing::info!("Receive folder set to: {}", path);
    Ok(())
}
