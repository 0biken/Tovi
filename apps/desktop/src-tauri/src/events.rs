//! Forwards `Node` events to the window as Tauri events.
//!
//! Event names and payloads are the frontend's contract; see `src/api.ts`.

use crate::commands::{direction_str, short, transfer_id_hex};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast::{error::RecvError, Receiver};
use tovi_core::node::{Event, TransferOutcome};

#[derive(Serialize, Clone)]
struct PairingRequest {
    request_id: u64,
    device_id: String,
    device_name: String,
    platform: String,
}

#[derive(Serialize, Clone)]
struct Paired {
    device_id: String,
    device_name: String,
    platform: String,
}

#[derive(Serialize, Clone)]
struct PairingFailed {
    device_id: String,
    short_id: String,
    error: String,
}

#[derive(Serialize, Clone)]
struct RequestExpired {
    request_id: u64,
}

#[derive(Serialize, Clone)]
struct Incoming {
    request_id: u64,
    transfer_id: String,
    device_name: String,
    file_name: String,
    file_size: u64,
}

#[derive(Serialize, Clone)]
struct Started {
    transfer_id: String,
    direction: &'static str,
    device_id: String,
    device_name: String,
    file_name: String,
    file_size: u64,
    resuming: bool,
}

#[derive(Serialize, Clone)]
struct ProgressPayload {
    transfer_id: String,
    direction: &'static str,
    done: u64,
    total: u64,
    bytes_per_sec: u64,
}

#[derive(Serialize, Clone)]
struct Reconnecting {
    transfer_id: String,
    error: String,
}

#[derive(Serialize, Clone)]
struct Finished {
    transfer_id: String,
    direction: &'static str,
    device_id: String,
    device_name: String,
    file_name: String,
    /// "completed" | "failed" | "interrupted"
    status: &'static str,
    /// Saved file, for completed receives
    path: Option<String>,
    size: Option<u64>,
    resumed_bytes: Option<u64>,
    error: Option<String>,
}

pub async fn forward(app: AppHandle, mut events: Receiver<Event>) {
    loop {
        match events.recv().await {
            Ok(event) => emit(&app, event),
            // Progress is lossy anyway; keep going
            Err(RecvError::Lagged(skipped)) => {
                tracing::warn!("UI fell behind; skipped {skipped} events")
            }
            Err(RecvError::Closed) => break,
        }
    }
}

fn emit(app: &AppHandle, event: Event) {
    let sent = match event {
        Event::PairingRequested {
            request_id,
            request,
        } => app.emit(
            "pairing:request",
            PairingRequest {
                request_id,
                device_id: request.device_id.to_string(),
                device_name: request.device_name,
                platform: request.platform,
            },
        ),
        Event::Paired { device } => app.emit(
            "pairing:done",
            Paired {
                device_id: device.id.to_string(),
                device_name: device.name,
                platform: device.platform,
            },
        ),
        Event::PairingFailed { device_id, error } => app.emit(
            "pairing:failed",
            PairingFailed {
                device_id: device_id.to_string(),
                short_id: short(&device_id),
                error,
            },
        ),
        Event::IncomingOffer {
            request_id,
            transfer_id,
            from,
            file_name,
            file_size,
        } => app.emit(
            "transfer:incoming",
            Incoming {
                request_id,
                transfer_id: transfer_id_hex(&transfer_id),
                device_name: from.name,
                file_name,
                file_size,
            },
        ),
        Event::RequestExpired { request_id } => {
            app.emit("request:expired", RequestExpired { request_id })
        }
        Event::TransferStarted {
            transfer_id,
            direction,
            device,
            file_name,
            file_size,
            resuming,
        } => app.emit(
            "transfer:started",
            Started {
                transfer_id: transfer_id_hex(&transfer_id),
                direction: direction_str(direction),
                device_id: device.id.to_string(),
                device_name: device.name,
                file_name,
                file_size,
                resuming,
            },
        ),
        Event::TransferProgress {
            transfer_id,
            direction,
            done,
            total,
            bytes_per_sec,
        } => app.emit(
            "transfer:progress",
            ProgressPayload {
                transfer_id: transfer_id_hex(&transfer_id),
                direction: direction_str(direction),
                done,
                total,
                bytes_per_sec,
            },
        ),
        Event::TransferReconnecting { transfer_id, error } => app.emit(
            "transfer:reconnecting",
            Reconnecting {
                transfer_id: transfer_id_hex(&transfer_id),
                error,
            },
        ),
        Event::TransferFinished {
            transfer_id,
            direction,
            device,
            file_name,
            outcome,
        } => {
            let mut finished = Finished {
                transfer_id: transfer_id_hex(&transfer_id),
                direction: direction_str(direction),
                device_id: device.id.to_string(),
                device_name: device.name,
                file_name,
                status: "completed",
                path: None,
                size: None,
                resumed_bytes: None,
                error: None,
            };
            match outcome {
                TransferOutcome::Completed {
                    path,
                    size,
                    resumed_bytes,
                    ..
                } => {
                    finished.path = path.map(|p| p.to_string_lossy().into_owned());
                    finished.size = Some(size);
                    finished.resumed_bytes = Some(resumed_bytes);
                }
                TransferOutcome::Failed { error } => {
                    finished.status = "failed";
                    finished.error = Some(error);
                }
                TransferOutcome::Interrupted { error } => {
                    finished.status = "interrupted";
                    finished.error = Some(error);
                }
            }
            app.emit("transfer:finished", finished)
        }
        Event::DevicesChanged => app.emit("devices:changed", ()),
    };
    if let Err(e) = sent {
        tracing::warn!("could not send an event to the window: {e}");
    }
}
