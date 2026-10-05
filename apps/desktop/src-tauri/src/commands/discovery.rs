use serde::Serialize;
use tovi_core::discovery::DiscoveryManager;

#[derive(Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub address: String,
    pub port: u16,
}

/// Returns all TOVI peers currently visible on the local network.
#[tauri::command]
pub async fn list_devices() -> Result<Vec<Device>, String> {
    let manager = DiscoveryManager::new().map_err(|e| e.to_string())?;
    let receiver = manager.browse().map_err(|e| e.to_string())?;

    let mut devices = Vec::new();

    // Collect events for up to 1 second then return snapshot
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        if let Ok(event) = receiver.try_recv() {
            use mdns_sd::ServiceEvent;
            if let ServiceEvent::ServiceResolved(info) = event {
                let address = info
                    .get_addresses()
                    .iter()
                    .next()
                    .map(|a| a.to_string())
                    .unwrap_or_default();

                let platform = info
                    .get_property_val_str("platform")
                    .unwrap_or("unknown")
                    .to_string();

                devices.push(Device {
                    id: info.get_fullname().to_string(),
                    name: info.get_hostname().trim_end_matches(".local.").to_string(),
                    platform,
                    address,
                    port: info.get_port(),
                });
            }
        }
    }

    Ok(devices)
}
