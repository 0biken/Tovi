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
use std::collections::{BTreeSet, HashMap};
use std::fmt::{self, Write};
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

pub const SERVICE_TYPE: &str = "_tovi._udp.local.";

/// How long a browse listens for answers. Discovery is on demand only; we
/// never browse continuously.
pub const BROWSE_WINDOW: Duration = Duration::from_secs(3);

/// The most addresses a browse returns, so a noisy network can't make a
/// connect attempt try an unbounded number of them.
pub const MAX_DISCOVERED_ADDRESSES: usize = 16;

/// How long to wait for the daemon to confirm an unregister, shutdown or
/// stop-browse; keeps it from logging about a receiver that went away.
const DAEMON_REPLY_WAIT: Duration = Duration::from_millis(250);

pub type BrowseFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>>> + Send + 'a>>;

/// Platform-pluggable LAN discovery.
pub trait Discovery: Send + Sync + fmt::Debug {
    /// Start advertising this device at `port`; replaces any earlier advert
    fn advertise(&self, port: u16) -> Result<()>;
    fn stop_advertising(&self);
    /// TOVI services seen within `window`: IPv4, deduplicated, sorted
    fn browse(&self, window: Duration) -> BrowseFuture<'_>;
}

/// Desktop discovery over `mdns-sd`.
pub struct MdnsDiscovery {
    daemon: ServiceDaemon,
    instance_id: String,
    registered: Mutex<Option<String>>,
    /// mdns-sd keeps one querier per service type, so browses must not overlap
    browse_lock: tokio::sync::Mutex<()>,
}

impl fmt::Debug for MdnsDiscovery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MdnsDiscovery")
            .field("instance_id", &self.instance_id)
            .finish_non_exhaustive()
    }
}

impl MdnsDiscovery {
    /// Initialize the mDNS daemon with a fresh random instance ID
    pub fn new() -> Result<Self> {
        Ok(Self {
            daemon: ServiceDaemon::new()?,
            instance_id: random_instance_id(),
            registered: Mutex::new(None),
            browse_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// The opaque ID this device advertises under
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    fn registered(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.registered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn own_fullname(&self) -> String {
        format!("{}.{SERVICE_TYPE}", self.instance_id)
    }
}

impl Discovery for MdnsDiscovery {
    fn advertise(&self, port: u16) -> Result<()> {
        self.stop_advertising();
        let info = service_info(&self.instance_id, port)?;
        let fullname = info.get_fullname().to_string();
        self.daemon.register(info)?;
        *self.registered() = Some(fullname);
        tracing::info!("Advertising TOVI service on port {port}");
        Ok(())
    }

    fn stop_advertising(&self) {
        if let Some(fullname) = self.registered().take() {
            // Wait for the goodbye so the daemon doesn't log a failed reply
            if let Ok(reply) = self.daemon.unregister(&fullname) {
                let _ = reply.recv_timeout(DAEMON_REPLY_WAIT);
            }
        }
    }

    fn browse(&self, window: Duration) -> BrowseFuture<'_> {
        Box::pin(async move {
            let own = self.own_fullname();
            // Serialize: a second concurrent browse would replace the first's
            // querier, and either one stopping would end the other's.
            let _serial = self.browse_lock.lock().await;
            let receiver = self.daemon.browse(SERVICE_TYPE)?;
            // Stops the browse even if this future is dropped mid-window.
            let mut _stop = StopBrowse(&self.daemon, true);
            let deadline = Instant::now() + window;
            let mut found = Vec::new();
            loop {
                let event = match tokio::time::timeout_at(deadline, receiver.recv_async()).await {
                    Err(_elapsed) => break,
                    Ok(Ok(event)) => event,
                    Ok(Err(_closed)) => anyhow::bail!("mDNS daemon stopped during browse"),
                };
                if let ServiceEvent::ServiceResolved(info) = event {
                    if info.get_fullname() == own {
                        continue;
                    }
                    found.extend(info.get_addresses().iter().map(|ip| (*ip, info.get_port())));
                }
            }
            // Stop now and wait for the daemon's confirmation, so it doesn't
            // log about answering a receiver we've already dropped
            if self.daemon.stop_browse(SERVICE_TYPE).is_ok() {
                _stop.disarm();
                let _ = tokio::time::timeout(Duration::from_millis(200), async {
                    while let Ok(event) = receiver.recv_async().await {
                        if matches!(event, ServiceEvent::SearchStopped(_)) {
                            break;
                        }
                    }
                })
                .await;
            }
            Ok(local_addresses(found))
        })
    }
}

/// Stops the service-type browse on drop, including on cancellation.
struct StopBrowse<'a>(&'a ServiceDaemon, bool);

impl StopBrowse<'_> {
    /// The browse was stopped explicitly; nothing left to do on drop
    fn disarm(&mut self) {
        self.1 = false;
    }
}

impl Drop for StopBrowse<'_> {
    fn drop(&mut self) {
        if self.1 {
            let _ = self.0.stop_browse(SERVICE_TYPE);
        }
    }
}

/// Keep only IPv4 addresses a LAN peer can have (private, link-local or
/// loopback), sorted, deduplicated and capped at [`MAX_DISCOVERED_ADDRESSES`]
fn local_addresses(found: impl IntoIterator<Item = (IpAddr, u16)>) -> Vec<SocketAddr> {
    let kept: BTreeSet<SocketAddr> = found
        .into_iter()
        .filter_map(|(ip, port)| match ip {
            IpAddr::V4(v4) if v4.is_private() || v4.is_link_local() || v4.is_loopback() => {
                Some(SocketAddr::new(ip, port))
            }
            _ => None,
        })
        .collect();
    kept.into_iter().take(MAX_DISCOVERED_ADDRESSES).collect()
}

impl Drop for MdnsDiscovery {
    fn drop(&mut self) {
        self.stop_advertising();
        if let Ok(reply) = self.daemon.shutdown() {
            let _ = reply.recv_timeout(DAEMON_REPLY_WAIT);
        }
    }
}

/// The anonymous advert: random instance name, TXT holds only the version.
fn service_info(instance_id: &str, port: u16) -> Result<ServiceInfo> {
    let host_name = format!("tovi-{instance_id}.local.");
    let mut properties = HashMap::new();
    properties.insert("v".to_string(), "1".to_string());
    Ok(ServiceInfo::new(
        SERVICE_TYPE,
        instance_id,
        &host_name,
        "", // No fixed IPs; filled in by enable_addr_auto() below
        port,
        properties,
    )?
    .enable_addr_auto())
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

    fn v4(a: u8, b: u8, c: u8, d: u8, port: u16) -> SocketAddr {
        SocketAddr::from(([a, b, c, d], port))
    }

    #[test]
    fn keeps_only_local_ipv4_sorted_and_deduplicated() {
        let ips: Vec<(IpAddr, u16)> = vec![
            (IpAddr::from([8, 8, 8, 8]), 1),
            (IpAddr::from([192, 168, 1, 5]), 2),
            (IpAddr::from([192, 168, 1, 5]), 2),
            (IpAddr::from([10, 0, 0, 1]), 3),
            (IpAddr::from([169, 254, 1, 1]), 4),
            (IpAddr::from([127, 0, 0, 1]), 5),
            (IpAddr::from([0u16, 0, 0, 0, 0, 0, 0, 1]), 6),
        ];
        let kept = local_addresses(ips);
        assert_eq!(
            kept,
            vec![
                v4(10, 0, 0, 1, 3),
                v4(127, 0, 0, 1, 5),
                v4(169, 254, 1, 1, 4),
                v4(192, 168, 1, 5, 2),
            ]
        );
    }

    #[test]
    fn caps_discovered_addresses() {
        let ips: Vec<(IpAddr, u16)> = (0..40u16)
            .map(|i| (IpAddr::from([192, 168, 1, 1]), 1000 + i))
            .collect();
        assert_eq!(local_addresses(ips).len(), MAX_DISCOVERED_ADDRESSES);
    }

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

    #[test]
    fn advert_carries_only_the_version() {
        let info = service_info("0123456789abcdef", 48210).unwrap();
        assert_eq!(info.get_properties().len(), 1);
        assert_eq!(info.get_property_val_str("v"), Some("1"));
        assert_eq!(info.get_port(), 48210);
        assert_eq!(info.get_fullname(), "0123456789abcdef._tovi._udp.local.");
    }

    #[tokio::test]
    #[ignore = "needs multicast"]
    async fn browse_returns_within_window() {
        let d = MdnsDiscovery::new().unwrap();
        let start = std::time::Instant::now();
        d.browse(Duration::from_millis(500)).await.unwrap();
        assert!(start.elapsed() < Duration::from_millis(1500));
    }

    #[tokio::test]
    #[ignore = "needs multicast"]
    async fn two_daemons_find_each_other() {
        let a = MdnsDiscovery::new().unwrap();
        let b = MdnsDiscovery::new().unwrap();
        a.advertise(50001).unwrap();
        let found = b.browse(BROWSE_WINDOW).await.unwrap();
        assert!(found.iter().any(|addr| addr.port() == 50001), "{found:?}");
    }

    #[tokio::test]
    #[ignore = "needs multicast"]
    async fn concurrent_browses_both_complete() {
        let a = MdnsDiscovery::new().unwrap();
        let b = MdnsDiscovery::new().unwrap();
        b.advertise(50002).unwrap();
        let start = std::time::Instant::now();
        let (r1, r2) = tokio::join!(a.browse(BROWSE_WINDOW), a.browse(BROWSE_WINDOW));
        for found in [r1.unwrap(), r2.unwrap()] {
            assert!(found.iter().any(|addr| addr.port() == 50002), "{found:?}");
        }
        // serialized: two full windows back to back
        assert!(start.elapsed() >= BROWSE_WINDOW * 2);
    }

    #[tokio::test]
    #[ignore = "needs multicast"]
    async fn cancelled_browse_stops_querying_enough_for_next_browse() {
        let a = MdnsDiscovery::new().unwrap();
        let b = MdnsDiscovery::new().unwrap();
        b.advertise(50003).unwrap();
        // Drop a browse mid-window; the lock and the browse must be released.
        let _ = tokio::time::timeout(Duration::from_millis(100), a.browse(BROWSE_WINDOW)).await;
        let start = std::time::Instant::now();
        let found = a.browse(BROWSE_WINDOW).await.unwrap();
        assert!(start.elapsed() < BROWSE_WINDOW + Duration::from_millis(1000));
        assert!(found.iter().any(|addr| addr.port() == 50003), "{found:?}");
    }
}
