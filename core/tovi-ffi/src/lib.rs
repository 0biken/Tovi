//! Bindings over [`tovi_core::node::Node`] for the mobile apps (decisions.md D4).
//!
//! A thin layer, like the desktop app's Tauri commands: [`ToviNode`] owns
//! the node and the Tokio runtime it runs on, takes and returns flat records
//! with IDs as hex strings, and delivers events to a foreign
//! [`EventListener`]. Async methods become Kotlin `suspend fun`s; their work
//! runs on this crate's runtime, so the caller's executor only waits.

mod error;
mod events;
mod types;

pub use error::ToviError;
pub use events::{EventListener, NodeEvent, TransferOutcome};
pub use types::{
    Device, Direction, LocalId, NodeConfig, PairingCode, SendResult, TransferRecord, TransferStatus,
};

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use tovi_core::identity::DeviceId;
use tovi_core::node::{self, Node};
use tovi_core::protocol::MAX_DEVICE_NAME_LEN;
use tovi_core::transfer::transfer_id_hex;

uniffi::setup_scaffolding!();

/// A running TOVI device. Create one per app process and keep it for the
/// process's lifetime; call [`ToviNode::shutdown`] before letting it go.
#[derive(uniffi::Object)]
pub struct ToviNode {
    node: Node,
    /// Always Some until dropped
    runtime: Option<Runtime>,
    forward_task: Mutex<Option<JoinHandle<()>>>,
}

#[uniffi::export]
impl ToviNode {
    /// Load or create this device's identity and database and start
    /// listening. Blocks briefly (disk and socket setup): call it off the
    /// main thread.
    #[uniffi::constructor]
    pub fn start(
        config: NodeConfig,
        listener: Arc<dyn EventListener>,
    ) -> Result<Arc<Self>, ToviError> {
        let default_receive_dir = PathBuf::from(&config.default_receive_dir);
        if !default_receive_dir.is_absolute() {
            return Err(ToviError::invalid(
                "the receive folder must be a full path, not a relative one",
            ));
        }
        let device_name: String = config
            .device_name
            .chars()
            .take(MAX_DEVICE_NAME_LEN)
            .collect();
        let mut core_config =
            node::NodeConfig::new(config.data_dir, device_name, default_receive_dir);
        if let Some(listen) = config.listen {
            core_config.listen = listen
                .parse::<SocketAddr>()
                .map_err(|e| ToviError::invalid(format!("bad listen address {listen:?}: {e}")))?;
        }

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("tovi")
            .build()
            .map_err(|e| ToviError::from(anyhow::Error::new(e).context("starting runtime")))?;
        let (node, events) = runtime.block_on(Node::start(core_config))?;
        let forward_task = runtime.spawn(events::forward(events, listener));
        Ok(Arc::new(Self {
            node,
            runtime: Some(runtime),
            forward_task: Mutex::new(Some(forward_task)),
        }))
    }

    // ------------------------------------------------------------ identity

    pub fn local_id(&self) -> LocalId {
        LocalId {
            device_id: self.node.device_id().to_string(),
            device_name: self.node.device_name().to_string(),
        }
    }

    /// The UDP port this device listens on
    pub fn port(&self) -> Result<u16, ToviError> {
        Ok(self.node.local_addr()?.port())
    }

    // ------------------------------------------------------------- pairing

    /// Open a pairing session at this device's network addresses (valid for
    /// 60 seconds)
    pub fn new_pairing_code(&self) -> Result<PairingCode, ToviError> {
        let _rt = self.runtime().enter();
        let code = self.node.new_pairing_code()?;
        Ok(PairingCode {
            payload: code.to_uri()?,
            expires_at: code.expires_at,
        })
    }

    /// Pair with the device whose `tovi://pair/...` code was scanned or pasted
    pub async fn pair(&self, code: String) -> Result<Device, ToviError> {
        let node = self.node.clone();
        let device = self.run(async move { Ok(node.pair(&code).await?) }).await?;
        self.devices()?
            .into_iter()
            .find(|d| d.id == device.id.to_string())
            .ok_or_else(|| ToviError::from(anyhow::anyhow!("paired device was not saved")))
    }

    /// Answer a `PairingRequested` or `IncomingOffer` event. Returns false
    /// if the request already expired or was answered.
    pub fn respond(&self, request_id: u64, allow: bool) -> bool {
        self.node.respond(request_id, allow)
    }

    // ------------------------------------------------------------- devices

    /// Paired devices, oldest pairing first
    pub fn devices(&self) -> Result<Vec<Device>, ToviError> {
        Ok(self.node.devices()?.into_iter().map(Device::from).collect())
    }

    /// Stop trusting a device; it must pair again before it can send.
    /// Returns whether it was paired.
    pub fn forget(&self, device_id: String) -> Result<bool, ToviError> {
        let id = parse_device_id(&device_id)?;
        Ok(self.node.forget(&id)?)
    }

    // ----------------------------------------------------------- transfers

    /// Send a file to a paired device. Returns once the receiver has
    /// verified and saved it; progress arrives as events meanwhile.
    pub async fn send_file(
        &self,
        device_id: String,
        path: String,
    ) -> Result<SendResult, ToviError> {
        let id = parse_device_id(&device_id)?;
        let node = self.node.clone();
        let report = self
            .run(async move { Ok(node.send_file(&id, Path::new(&path)).await?) })
            .await?;
        Ok(SendResult {
            transfer_id: transfer_id_hex(&report.transfer_id),
            size: report.size,
        })
    }

    /// Recent transfers, newest first
    pub fn history(&self, limit: u32) -> Result<Vec<TransferRecord>, ToviError> {
        let names: HashMap<_, _> = self
            .node
            .devices()?
            .into_iter()
            .map(|d| (d.device.id, d.device.name))
            .collect();
        Ok(self
            .node
            .history(limit as usize)?
            .into_iter()
            .map(|t| {
                let name = names.get(&t.device_id).cloned();
                TransferRecord::new(t, name)
            })
            .collect())
    }

    // ------------------------------------------------------------ settings

    /// Where received files are saved
    pub fn receive_dir(&self) -> String {
        self.node.receive_dir().to_string_lossy().into_owned()
    }

    /// Save a new receive folder (a full path); applies to the next transfer
    pub fn set_receive_dir(&self, path: String) -> Result<(), ToviError> {
        let path = Path::new(&path);
        if !path.is_absolute() {
            return Err(ToviError::invalid(
                "the receive folder must be a full path, not a relative one",
            ));
        }
        let _rt = self.runtime().enter();
        Ok(self.node.set_receive_dir(path)?)
    }

    /// Whether files from paired devices are saved without asking (default: yes)
    pub fn auto_accept(&self) -> bool {
        self.node.auto_accept()
    }

    pub fn set_auto_accept(&self, enabled: bool) -> Result<(), ToviError> {
        Ok(self.node.set_auto_accept(enabled)?)
    }

    // ----------------------------------------------------------- lifecycle

    /// Stop accepting connections, tell connected devices we're going, and
    /// close the database. No more events are delivered afterwards.
    pub async fn shutdown(&self) {
        let node = self.node.clone();
        let _ = self
            .run(async move {
                node.shutdown().await;
                Ok(())
            })
            .await;
        if let Some(task) = self.forward_task.lock().unwrap().take() {
            task.abort();
        }
    }
}

impl ToviNode {
    fn runtime(&self) -> &Runtime {
        self.runtime.as_ref().expect("runtime lives until drop")
    }

    /// Run `work` on this node's runtime; the caller's executor just waits
    async fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, ToviError>> + Send + 'static,
    ) -> Result<T, ToviError> {
        self.runtime()
            .spawn(work)
            .await
            .map_err(|e| ToviError::from(anyhow::anyhow!("background task failed: {e}")))?
    }
}

impl Drop for ToviNode {
    fn drop(&mut self) {
        // Dropping a runtime blocks, and panics inside async code; this may
        // run on any thread (a Kotlin cleaner, a coroutine), so don't wait
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

fn parse_device_id(id: &str) -> Result<DeviceId, ToviError> {
    id.parse().map_err(ToviError::invalid)
}
