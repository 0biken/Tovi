//! Trusted devices: the devices this one has paired with.
//!
//! [`crate::storage::Store`] keeps them on disk; [`MemoryTrustStore`] is for
//! tests and throwaway sessions.

use crate::identity::DeviceId;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedDevice {
    pub id: DeviceId,
    pub name: String,
    pub platform: String,
}

pub trait TrustStore: Send + Sync {
    /// Fails closed: if the store can't be read, the device is not trusted
    fn is_trusted(&self, id: &DeviceId) -> bool;
    /// Add or update a trusted device
    fn add(&self, device: TrustedDevice) -> Result<()>;
    /// Revoke / forget a device. Returns whether it was trusted.
    fn remove(&self, id: &DeviceId) -> Result<bool>;
    fn list(&self) -> Result<Vec<TrustedDevice>>;

    fn get(&self, id: &DeviceId) -> Result<Option<TrustedDevice>> {
        Ok(self.list()?.into_iter().find(|d| d.id == *id))
    }
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

    fn add(&self, device: TrustedDevice) -> Result<()> {
        self.devices.lock().unwrap().insert(device.id, device);
        Ok(())
    }

    fn remove(&self, id: &DeviceId) -> Result<bool> {
        Ok(self.devices.lock().unwrap().remove(id).is_some())
    }

    fn list(&self) -> Result<Vec<TrustedDevice>> {
        Ok(self.devices.lock().unwrap().values().cloned().collect())
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
        store.add(device.clone()).unwrap();
        assert!(store.is_trusted(&id));
        assert_eq!(store.list().unwrap(), vec![device.clone()]);
        assert_eq!(store.get(&id).unwrap(), Some(device));

        assert!(store.remove(&id).unwrap());
        assert!(!store.is_trusted(&id));
        assert!(!store.remove(&id).unwrap());
    }
}
