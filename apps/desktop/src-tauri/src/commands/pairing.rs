use super::devices::Device;
use super::{message, CommandResult};
use serde::Serialize;
use tauri::State;
use tovi_core::node::Node;

#[derive(Serialize)]
pub struct LocalIdInfo {
    pub device_id: String,
    pub device_name: String,
}

#[derive(Serialize)]
pub struct QrPayload {
    /// The `tovi://pair/...` link; render it as a QR code
    pub payload: String,
    /// Unix seconds
    pub expires_at: u64,
}

/// This device's identity
#[tauri::command]
pub fn get_local_id(node: State<'_, Node>) -> LocalIdInfo {
    LocalIdInfo {
        device_id: node.device_id().to_string(),
        device_name: node.device_name().to_string(),
    }
}

/// Open a pairing session and return its code (valid for 60 seconds)
#[tauri::command]
pub fn generate_qr(node: State<'_, Node>) -> CommandResult<QrPayload> {
    let code = node.new_pairing_code().map_err(message)?;
    Ok(QrPayload {
        payload: code.to_uri().map_err(message)?,
        expires_at: code.expires_at,
    })
}

/// Pair with another device using its code (pasted link)
#[tauri::command]
pub async fn pair_with_code(node: State<'_, Node>, code: String) -> CommandResult<Device> {
    let device = node.pair(&code).await.map_err(message)?;
    let address = node
        .devices()
        .map_err(message)?
        .into_iter()
        .find(|d| d.device.id == device.id)
        .and_then(|d| d.addresses.first().map(|a| a.to_string()));
    Ok(Device {
        id: device.id.to_string(),
        name: device.name,
        platform: device.platform,
        paired_at: 0,
        last_seen: None,
        address,
    })
}

/// Answer a `pairing:request` or `transfer:incoming` event. Returns false if
/// the request already expired.
#[tauri::command]
pub fn respond(node: State<'_, Node>, request_id: u64, allow: bool) -> bool {
    node.respond(request_id, allow)
}
