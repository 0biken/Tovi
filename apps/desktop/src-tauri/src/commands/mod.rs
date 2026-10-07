//! Tauri commands: thin wrappers over `tovi_core::node::Node`.
//!
//! Errors reach the frontend as their full message (`{:#}`), since the UI
//! shows them to the user.

pub mod devices;
pub mod pairing;
pub mod settings;
pub mod transfer;

use tovi_core::identity::DeviceId;
use tovi_core::storage::Direction;
use tovi_core::transfer::TransferId;

/// Result type for commands: the error is shown to the user
pub type CommandResult<T> = Result<T, String>;

pub fn message(e: anyhow::Error) -> String {
    format!("{e:#}")
}

pub fn parse_device_id(id: &str) -> CommandResult<DeviceId> {
    id.parse().map_err(message)
}

/// First 8 hex characters of a device ID, for display
pub fn short(id: &DeviceId) -> String {
    id.short()
}

pub fn transfer_id_hex(id: &TransferId) -> String {
    tovi_core::transfer::transfer_id_hex(id)
}

pub fn direction_str(direction: Direction) -> &'static str {
    direction.as_str()
}
