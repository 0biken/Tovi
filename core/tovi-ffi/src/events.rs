//! `Node` events as one flat enum, delivered to a Kotlin callback.
//!
//! Mirrors the desktop app's `events.rs`, variant for variant.

use crate::types::Direction;
use std::sync::Arc;
use tokio::sync::broadcast::{error::RecvError, Receiver};
use tovi_core::node::{self, Event};
use tovi_core::transfer::transfer_id_hex;

/// Implemented in Kotlin. Called from a background thread, one event at a
/// time; return quickly (hand the event to a Flow or a channel).
#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, event: NodeEvent);
}

#[derive(uniffi::Enum, Debug, Clone)]
pub enum NodeEvent {
    /// A device scanned our code; answer with `respond(request_id, ...)`
    PairingRequested {
        request_id: u64,
        device_id: String,
        device_name: String,
        platform: String,
    },
    Paired {
        device_id: String,
        device_name: String,
        platform: String,
    },
    /// An unpaired device connected and did not complete pairing
    PairingFailed {
        device_id: String,
        short_id: String,
        error: String,
    },
    /// A paired device offers a file and auto-accept is off; answer with
    /// `respond(request_id, ...)`
    IncomingOffer {
        request_id: u64,
        transfer_id: String,
        device_id: String,
        device_name: String,
        file_name: String,
        file_size: u64,
    },
    /// A request timed out unanswered and was declined
    RequestExpired { request_id: u64 },
    TransferStarted {
        transfer_id: String,
        direction: Direction,
        device_id: String,
        device_name: String,
        file_name: String,
        file_size: u64,
        /// An unfinished earlier send of the same file is being resumed
        resuming: bool,
    },
    TransferProgress {
        transfer_id: String,
        direction: Direction,
        done: u64,
        total: u64,
        bytes_per_sec: u64,
    },
    /// The connection dropped; the sender is reconnecting to resume
    TransferReconnecting { transfer_id: String, error: String },
    TransferFinished {
        transfer_id: String,
        direction: Direction,
        device_id: String,
        device_name: String,
        file_name: String,
        outcome: TransferOutcome,
    },
    /// The list of paired devices changed
    DevicesChanged,
    /// The listener fell behind and `skipped` events were dropped; reload
    /// devices and history
    Lagged { skipped: u64 },
}

#[derive(uniffi::Enum, Debug, Clone)]
pub enum TransferOutcome {
    Completed {
        /// Where it was saved (received files only)
        path: Option<String>,
        size: u64,
        /// Bytes already present from an earlier attempt (received files only)
        resumed_bytes: u64,
    },
    /// Over for good: refused, declined, changed file, failed integrity check
    Failed { error: String },
    /// The connection was lost; it can resume later
    Interrupted { error: String },
}

pub(crate) async fn forward(mut events: Receiver<Event>, listener: Arc<dyn EventListener>) {
    loop {
        match events.recv().await {
            Ok(event) => listener.on_event(event.into()),
            Err(RecvError::Lagged(skipped)) => {
                tracing::warn!("event listener fell behind; skipped {skipped} events");
                listener.on_event(NodeEvent::Lagged { skipped });
            }
            Err(RecvError::Closed) => break,
        }
    }
}

impl From<Event> for NodeEvent {
    fn from(event: Event) -> Self {
        match event {
            Event::PairingRequested {
                request_id,
                request,
            } => Self::PairingRequested {
                request_id,
                device_id: request.device_id.to_string(),
                device_name: request.device_name,
                platform: request.platform,
            },
            Event::Paired { device } => Self::Paired {
                device_id: device.id.to_string(),
                device_name: device.name,
                platform: device.platform,
            },
            Event::PairingFailed { device_id, error } => Self::PairingFailed {
                device_id: device_id.to_string(),
                short_id: device_id.short(),
                error,
            },
            Event::IncomingOffer {
                request_id,
                transfer_id,
                from,
                file_name,
                file_size,
            } => Self::IncomingOffer {
                request_id,
                transfer_id: transfer_id_hex(&transfer_id),
                device_id: from.id.to_string(),
                device_name: from.name,
                file_name,
                file_size,
            },
            Event::RequestExpired { request_id } => Self::RequestExpired { request_id },
            Event::TransferStarted {
                transfer_id,
                direction,
                device,
                file_name,
                file_size,
                resuming,
            } => Self::TransferStarted {
                transfer_id: transfer_id_hex(&transfer_id),
                direction: direction.into(),
                device_id: device.id.to_string(),
                device_name: device.name,
                file_name,
                file_size,
                resuming,
            },
            Event::TransferProgress {
                transfer_id,
                direction,
                done,
                total,
                bytes_per_sec,
            } => Self::TransferProgress {
                transfer_id: transfer_id_hex(&transfer_id),
                direction: direction.into(),
                done,
                total,
                bytes_per_sec,
            },
            Event::TransferReconnecting { transfer_id, error } => Self::TransferReconnecting {
                transfer_id: transfer_id_hex(&transfer_id),
                error,
            },
            Event::TransferFinished {
                transfer_id,
                direction,
                device,
                file_name,
                outcome,
            } => Self::TransferFinished {
                transfer_id: transfer_id_hex(&transfer_id),
                direction: direction.into(),
                device_id: device.id.to_string(),
                device_name: device.name,
                file_name,
                outcome: match outcome {
                    node::TransferOutcome::Completed {
                        path,
                        size,
                        resumed_bytes,
                        ..
                    } => TransferOutcome::Completed {
                        path: path.map(|p| p.to_string_lossy().into_owned()),
                        size,
                        resumed_bytes,
                    },
                    node::TransferOutcome::Failed { error } => TransferOutcome::Failed { error },
                    node::TransferOutcome::Interrupted { error } => {
                        TransferOutcome::Interrupted { error }
                    }
                },
            },
            Event::DevicesChanged => Self::DevicesChanged,
        }
    }
}
