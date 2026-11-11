use super::{message, parse_device_id, CommandResult};
use serde::Serialize;
use tauri::State;
use tovi_core::node::Node;

/// A paired device
#[derive(Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    /// Unix seconds
    pub paired_at: u64,
    pub last_seen: Option<u64>,
    /// Best known address, e.g. "192.168.1.20:48210"
    pub address: Option<String>,
}

/// Paired devices, oldest pairing first
#[tauri::command]
pub fn list_devices(node: State<'_, Node>) -> CommandResult<Vec<Device>> {
    let devices = node.devices().map_err(message)?;
    Ok(devices
        .into_iter()
        .map(|d| Device {
            id: d.device.id.to_string(),
            name: d.device.name,
            platform: d.device.platform,
            paired_at: d.paired_at,
            last_seen: d.last_seen,
            address: d.addresses.first().map(|a| a.to_string()),
        })
        .collect())
}

/// Stop trusting a device; it must pair again before it can send
#[tauri::command]
pub fn forget_device(node: State<'_, Node>, device_id: String) -> CommandResult<bool> {
    let id = parse_device_id(&device_id)?;
    node.forget(&id).map_err(message)
}
