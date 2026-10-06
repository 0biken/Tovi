//! Tauri commands: thin wrappers over `tovi_core::node::Node`.
//!
//! Errors reach the frontend as their full message (`{:#}`), since the UI
//! shows them to the user.

pub mod devices;
pub mod pairing;
pub mod settings;
pub mod transfer;

use std::fmt::Write as _;
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
    id.to_string()[..8].to_string()
}

pub fn transfer_id_hex(id: &TransferId) -> String {
    let mut hex = String::with_capacity(32);
    for b in id {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

pub fn direction_str(direction: Direction) -> &'static str {
    match direction {
        Direction::Sent => "sent",
        Direction::Received => "received",
    }
}
