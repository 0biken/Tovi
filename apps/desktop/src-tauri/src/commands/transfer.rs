use serde::Serialize;

#[derive(Serialize)]
pub struct TransferRecord {
    pub id: String,
    pub file_name: String,
    pub file_size: u64,
    pub direction: String,  // "sent" | "received"
    pub device_name: String,
    pub status: String,     // "completed" | "failed" | "in_progress"
    pub created_at: u64,    // Unix timestamp
}

/// Send a file to a paired device.
#[tauri::command]
pub async fn send_file(
    app: tauri::AppHandle,
    device_id: String,
    path: String,
) -> Result<(), String> {
    // TODO: delegate to tovi-core transfer engine.
    // Progress updates should be emitted as Tauri events so the frontend
    // can update its progress ring in real time:
    //   app.emit_all("transfer:progress", ProgressPayload { ... })
    tracing::info!("Sending {} to {}", path, device_id);
    Ok(())
}

/// Returns local transfer history from SQLite.
#[tauri::command]
pub async fn list_transfers() -> Result<Vec<TransferRecord>, String> {
    // TODO: query tovi-core storage module
    Ok(vec![])
}
