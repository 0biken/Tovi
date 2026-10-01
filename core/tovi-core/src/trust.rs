//! Trusted devices: the devices this one has paired with.
//!
//! In memory for now; persistence moves to SQLite with the rest of local
//! storage (TASKS §9).

use crate::identity::DeviceId;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedDevice {
    pub id: DeviceId,
    pub name: String,
    pub platform: String,
}

pub trait TrustStore: Send + Sync {
    fn is_trusted(&self, id: &DeviceId) -> bool;
    /// Add or replace a trusted device
    fn add(&self, device: TrustedDevice);
    /// Revoke / forget a device. Returns whether it was trusted.
    fn remove(&self, id: &DeviceId) -> bool;
    fn list(&self) -> Vec<TrustedDevice>;
}

#[derive(Default)]
pub struct MemoryTrustStore {
    devices: Mutex<HashMap<DeviceId, TrustedDevice>>,
}

impl MemoryTrustStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TrustStore for MemoryTrustStore {
    fn is_trusted(&self, id: &DeviceId) -> bool {
        self.devices.lock().unwrap().contains_key(id)
    }

    fn add(&self, device: TrustedDevice) {
        self.devices.lock().unwrap().insert(device.id, device);
    }

    fn remove(&self, id: &DeviceId) -> bool {
        self.devices.lock().unwrap().remove(id).is_some()
    }

    fn list(&self) -> Vec<TrustedDevice> {
        self.devices.lock().unwrap().values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;

    #[test]
    fn add_list_and_revoke() {
        let store = MemoryTrustStore::new();
        let id = DeviceIdentity::generate_new().device_id();
        let device = TrustedDevice {
            id,
            name: "Pixel".into(),
            platform: "android".into(),
        };

        assert!(!store.is_trusted(&id));
        store.add(device.clone());
        assert!(store.is_trusted(&id));
        assert_eq!(store.list(), vec![device]);

        assert!(store.remove(&id));
        assert!(!store.is_trusted(&id));
        assert!(!store.remove(&id));
    }
}
