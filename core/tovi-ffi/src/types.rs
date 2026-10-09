//! Flat records and enums for the bindings. IDs cross as hex strings and
//! paths as strings, the same as in the desktop app's commands.

use tovi_core::storage::{self, DeviceDetails};
use tovi_core::transfer::transfer_id_hex;

/// How to start a [`crate::ToviNode`]
#[derive(uniffi::Record, Debug, Clone)]
pub struct NodeConfig {
    /// Holds the identity key and database; app-private storage
    pub data_dir: String,
    /// Shown to other devices; cut to 64 characters
    pub device_name: String,
    /// Receive folder until the user picks one; must be a full path
    pub default_receive_dir: String,
    /// Where to listen, e.g. "127.0.0.1:0"; None for the default port on all interfaces
    pub listen: Option<String>,
}

/// This device's identity
#[derive(uniffi::Record, Debug, Clone)]
pub struct LocalId {
    pub device_id: String,
    pub device_name: String,
}

/// A paired device
#[derive(uniffi::Record, Debug, Clone)]
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

impl From<DeviceDetails> for Device {
    fn from(d: DeviceDetails) -> Self {
        Self {
            id: d.device.id.to_string(),
            name: d.device.name,
            platform: d.device.platform,
            paired_at: d.paired_at,
            last_seen: d.last_seen,
            address: d.addresses.first().map(|a| a.to_string()),
        }
    }
}

/// A pairing code to show as a QR code
#[derive(uniffi::Record, Debug, Clone)]
pub struct PairingCode {
    /// The `tovi://pair/...` link
    pub payload: String,
    /// Unix seconds
    pub expires_at: u64,
}

#[derive(uniffi::Record, Debug, Clone)]
pub struct SendResult {
    pub transfer_id: String,
    pub size: u64,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Sent,
    Received,
}

impl From<storage::Direction> for Direction {
    fn from(d: storage::Direction) -> Self {
        match d {
            storage::Direction::Sent => Self::Sent,
            storage::Direction::Received => Self::Received,
        }
    }
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferStatus {
    InProgress,
    Completed,
    Failed,
}

/// One row of transfer history
#[derive(uniffi::Record, Debug, Clone)]
pub struct TransferRecord {
    pub id: String,
    pub file_name: String,
    pub file_size: u64,
    pub direction: Direction,
    pub device_id: String,
    /// The device's name, or its short ID if it was forgotten
    pub device_name: String,
    pub status: TransferStatus,
    /// Unix seconds
    pub created_at: u64,
    pub finished_at: Option<u64>,
    /// Source file (sent) or saved file (received)
    pub path: Option<String>,
    pub error: Option<String>,
}

impl TransferRecord {
    pub(crate) fn new(t: storage::TransferRecord, device_name: Option<String>) -> Self {
        Self {
            id: transfer_id_hex(&t.id),
            file_name: t.file_name,
            file_size: t.file_size,
            direction: t.direction.into(),
            device_name: device_name.unwrap_or_else(|| t.device_id.short()),
            device_id: t.device_id.to_string(),
            status: match t.status {
                storage::TransferStatus::InProgress => TransferStatus::InProgress,
                storage::TransferStatus::Completed => TransferStatus::Completed,
                storage::TransferStatus::Failed => TransferStatus::Failed,
            },
            created_at: t.started_at,
            finished_at: t.finished_at,
            path: t.path.map(|p| p.to_string_lossy().into_owned()),
            error: t.error,
        }
    }
}
