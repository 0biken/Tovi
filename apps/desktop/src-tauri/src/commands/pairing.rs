use serde::Serialize;

#[derive(Serialize)]
pub struct LocalIdInfo {
    pub device_id: String,
    pub device_name: String,
}

#[derive(Serialize)]
pub struct QrPayload {
    pub payload: String,   // base64-encoded QR content string
    pub expires_at: u64,   // Unix timestamp (seconds)
}

/// Returns this device's public identity (device_id and name).
#[tauri::command]
pub async fn get_local_id() -> Result<LocalIdInfo, String> {
    // TODO: load from tovi-core identity store
    Ok(LocalIdInfo {
        device_id: "placeholder-device-id".to_string(),
        device_name: hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "My Desktop".to_string()),
    })
}

/// Generates a short-lived QR pairing token for a phone to scan.
#[tauri::command]
pub async fn generate_qr() -> Result<QrPayload, String> {
    let expires_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 60; // 60-second validity window

    // TODO: generate real ephemeral TVP session token from tovi-core pairing module
    let raw_payload = format!(
        "{{\"v\":1,\"device\":\"placeholder-id\",\"expires\":{}}}",
        expires_at
    );
    let payload = base64::engine::general_purpose::STANDARD.encode(raw_payload.as_bytes());

    Ok(QrPayload { payload, expires_at })
}

/// Called by the frontend with a scanned QR code string to complete pairing.
#[tauri::command]
pub async fn pair_with_code(code: String) -> Result<(), String> {
    // TODO: delegate to tovi-core pairing module
    tracing::info!("Pairing with code: {}", code);
    Ok(())
}
