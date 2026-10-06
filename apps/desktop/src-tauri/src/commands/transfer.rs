use super::{direction_str, message, parse_device_id, short, transfer_id_hex, CommandResult};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use tauri::State;
use tovi_core::node::Node;
use tovi_core::storage::TransferStatus;

#[derive(Serialize)]
pub struct SendResult {
    pub transfer_id: String,
    pub size: u64,
}

#[derive(Serialize)]
pub struct TransferRecord {
    pub id: String,
    pub file_name: String,
    pub file_size: u64,
    /// "sent" | "received"
    pub direction: &'static str,
    pub device_id: String,
    pub device_name: String,
    /// "completed" | "failed" | "in_progress"
    pub status: &'static str,
    /// Unix seconds
    pub created_at: u64,
    pub finished_at: Option<u64>,
    /// Source file (sent) or saved file (received)
    pub path: Option<String>,
    pub error: Option<String>,
}

/// Send a file to a paired device. Resolves once the receiver has verified
/// and saved it; progress arrives as `transfer:*` events meanwhile.
#[tauri::command]
pub async fn send_file(
    node: State<'_, Node>,
    device_id: String,
    path: String,
) -> CommandResult<SendResult> {
    let id = parse_device_id(&device_id)?;
    let report = node
        .send_file(&id, Path::new(&path))
        .await
        .map_err(message)?;
    Ok(SendResult {
        transfer_id: transfer_id_hex(&report.transfer_id),
        size: report.size,
    })
}

/// Recent transfers, newest first
#[tauri::command]
pub fn list_transfers(
    node: State<'_, Node>,
    limit: Option<usize>,
) -> CommandResult<Vec<TransferRecord>> {
    let names: HashMap<_, _> = node
        .devices()
        .map_err(message)?
        .into_iter()
        .map(|d| (d.device.id, d.device.name))
        .collect();
    let history = node.history(limit.unwrap_or(200)).map_err(message)?;
    Ok(history
        .into_iter()
        .map(|t| TransferRecord {
            id: transfer_id_hex(&t.id),
            file_name: t.file_name,
            file_size: t.file_size,
            direction: direction_str(t.direction),
            device_id: t.device_id.to_string(),
            // A forgotten device keeps its history under its short ID
            device_name: names
                .get(&t.device_id)
                .cloned()
                .unwrap_or_else(|| short(&t.device_id)),
            status: match t.status {
                TransferStatus::Completed => "completed",
                TransferStatus::Failed => "failed",
                TransferStatus::InProgress => "in_progress",
            },
            created_at: t.started_at,
            finished_at: t.finished_at,
            path: t.path.map(|p| p.to_string_lossy().into_owned()),
            error: t.error,
        })
        .collect())
}
