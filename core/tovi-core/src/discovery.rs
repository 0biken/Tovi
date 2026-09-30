//! LAN device discovery (mDNS / DNS-SD, `_tovi._udp.local`).
//!
//! Discovery is platform-pluggable: desktop uses `mdns-sd` directly, while
//! Android (NSD) and iOS (Bonjour / Network framework) supply their own adapters.

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;

pub const SERVICE_TYPE: &str = "_tovi._udp.local.";

pub struct DiscoveryManager {
    daemon: ServiceDaemon,
}

impl DiscoveryManager {
    /// Initialize the mDNS daemon
    pub fn new() -> Result<Self> {
        let daemon = ServiceDaemon::new()?;
        Ok(Self { daemon })
    }

    /// Advertise this device on the local network
    pub fn advertise(&self, instance_name: &str, port: u16) -> Result<()> {
        let host_name = format!("{instance_name}.local.");

        let mut properties = HashMap::new();
        properties.insert("version".to_string(), "1".to_string());
        properties.insert("platform".to_string(), std::env::consts::OS.to_string());

        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            instance_name,
            &host_name,
            "", // No fixed IPs; filled in by enable_addr_auto() below
            port,
            properties,
        )?
        .enable_addr_auto();

        self.daemon.register(service_info)?;
        tracing::info!("Advertising TOVI service: {instance_name} on port {port}");
        Ok(())
    }

    /// Browse for other TOVI devices on the local network
    pub fn browse(&self) -> Result<mdns_sd::Receiver<ServiceEvent>> {
        let receiver = self.daemon.browse(SERVICE_TYPE)?;
        tracing::info!("Browsing for TOVI services...");
        Ok(receiver)
    }
}
