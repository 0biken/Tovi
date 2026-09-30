//! LAN device discovery (mDNS / DNS-SD, `_tovi._udp.local`).
//!
//! Discovery is platform-pluggable: desktop uses `mdns-sd` directly, while
//! Android (NSD) and iOS (Bonjour / Network framework) supply their own adapters.
//!
//! Privacy: nothing personal is broadcast. The instance and host names are a
//! random ID regenerated each run, so passive listeners can't learn the
//! device's name or track it across sessions. Display names are exchanged
//! only after the encrypted connection is established (TVP `HELLO`).

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use rand_core::{OsRng, RngCore};
use std::collections::HashMap;
use std::fmt::Write;

pub const SERVICE_TYPE: &str = "_tovi._udp.local.";

pub struct DiscoveryManager {
    daemon: ServiceDaemon,
    instance_id: String,
}

impl DiscoveryManager {
    /// Initialize the mDNS daemon with a fresh random instance ID
    pub fn new() -> Result<Self> {
        let daemon = ServiceDaemon::new()?;
        Ok(Self {
            daemon,
            instance_id: random_instance_id(),
        })
    }

    /// The opaque ID this device advertises under
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Advertise this device on the local network
    pub fn advertise(&self, port: u16) -> Result<()> {
        let host_name = format!("tovi-{}.local.", self.instance_id);

        let mut properties = HashMap::new();
        properties.insert("version".to_string(), "1".to_string());
        properties.insert("platform".to_string(), std::env::consts::OS.to_string());

        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &self.instance_id,
            &host_name,
            "", // No fixed IPs; filled in by enable_addr_auto() below
            port,
            properties,
        )?
        .enable_addr_auto();

        self.daemon.register(service_info)?;
        tracing::info!(
            "Advertising TOVI service: {} on port {port}",
            self.instance_id
        );
        Ok(())
    }

    /// Browse for other TOVI devices on the local network
    pub fn browse(&self) -> Result<mdns_sd::Receiver<ServiceEvent>> {
        let receiver = self.daemon.browse(SERVICE_TYPE)?;
        tracing::info!("Browsing for TOVI services...");
        Ok(receiver)
    }
}

/// 64 random bits as 16 lowercase hex chars — a valid DNS label with no
/// personal information in it.
fn random_instance_id() -> String {
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    let mut id = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(id, "{b:02x}");
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_id_is_valid_dns_label() {
        let id = random_instance_id();
        assert_eq!(id.len(), 16);
        assert!(id
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
    }

    #[test]
    fn instance_ids_are_unique() {
        assert_ne!(random_instance_id(), random_instance_id());
    }
}
