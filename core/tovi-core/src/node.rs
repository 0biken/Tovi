//! A running TOVI device: the one object apps drive.
//!
//! [`Node`] owns this device's identity, local database, QUIC endpoint,
//! pairing sessions and inbox. It accepts connections in the background:
//! unknown devices may only pair, paired devices may send files. Everything
//! that happens is reported as an [`Event`]; decisions that need the user
//! (pairing a new device, or accepting a file when auto-accept is off) arrive
//! as events with a `request_id`, answered with [`Node::respond`] and declined
//! automatically after [`NodeConfig::approval_timeout`].
//!
//! The CLI and the desktop app are thin layers over this; the mobile apps
//! will be too.

use crate::identity::{DeviceId, DeviceIdentity, FileKeyStore};
use crate::pairing::{self, PairingCode, PairingManager, PairingRequest};
use crate::protocol::{self, Hello, Message, TransferOffer};
use crate::storage::{DeviceDetails, Direction, Store, TransferRecord};
use crate::transfer::{
    self, ConnectionLost, Inbox, OutgoingTransfer, Progress, SendOptions, TransferId,
};
use crate::transport::{candidate_addresses, PeerConnection, QuicEndpoint};
use crate::trust::{TrustStore, TrustedDevice};
use anyhow::{anyhow, bail, Context, Result};
use quinn::{RecvStream, SendStream};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, oneshot};
use tokio::task::{JoinHandle, JoinSet};

/// Default UDP port (Tech doc §13). If it's taken, a free one is used instead.
pub const DEFAULT_PORT: u16 = 48210;
/// How long a pairing or file request waits for the user before declining
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);
/// Progress events per transfer are limited to roughly this rate
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const EVENT_BUFFER: usize = 256;

const SETTING_RECEIVE_DIR: &str = "receive_dir";
const SETTING_AUTO_ACCEPT: &str = "auto_accept";

#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Holds `identity.key` and `tovi.db`
    pub data_dir: PathBuf,
    pub device_name: String,
    /// Where to listen; `0.0.0.0:48210` by default
    pub listen: SocketAddr,
    /// Receive folder when the user hasn't chosen one
    pub default_receive_dir: PathBuf,
    /// Use this receive folder for this run only, without saving it
    pub receive_dir_override: Option<PathBuf>,
    /// Trust any device that proves it scanned a valid code, without asking.
    /// For automated testing only.
    pub auto_approve_pairing: bool,
    pub approval_timeout: Duration,
}

impl NodeConfig {
    pub fn new(
        data_dir: impl Into<PathBuf>,
        device_name: impl Into<String>,
        default_receive_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            data_dir: data_dir.into(),
            device_name: device_name.into(),
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), DEFAULT_PORT),
            default_receive_dir: default_receive_dir.into(),
            receive_dir_override: None,
            auto_approve_pairing: false,
            approval_timeout: APPROVAL_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    /// A device scanned our code and proved it; answer with [`Node::respond`]
    PairingRequested {
        request_id: u64,
        request: PairingRequest,
    },
    Paired {
        device: TrustedDevice,
    },
    /// An unpaired device connected and did not complete pairing
    PairingFailed {
        device_id: DeviceId,
        error: String,
    },
    /// A paired device offers a file and auto-accept is off; answer with
    /// [`Node::respond`]
    IncomingOffer {
        request_id: u64,
        transfer_id: TransferId,
        from: TrustedDevice,
        file_name: String,
        file_size: u64,
    },
    /// A request timed out unanswered and was declined
    RequestExpired {
        request_id: u64,
    },
    TransferStarted {
        transfer_id: TransferId,
        direction: Direction,
        device: TrustedDevice,
        file_name: String,
        file_size: u64,
        /// An unfinished earlier send of the same file is being resumed
        resuming: bool,
    },
    TransferProgress {
        transfer_id: TransferId,
        direction: Direction,
        done: u64,
        total: u64,
        bytes_per_sec: u64,
    },
    /// The connection dropped; the sender is reconnecting to resume
    TransferReconnecting {
        transfer_id: TransferId,
        error: String,
    },
    TransferFinished {
        transfer_id: TransferId,
        direction: Direction,
        device: TrustedDevice,
        file_name: String,
        outcome: TransferOutcome,
    },
    /// The list of paired devices changed
    DevicesChanged,
}

#[derive(Debug, Clone)]
pub enum TransferOutcome {
    Completed {
        /// Where it was saved (received files only)
        path: Option<PathBuf>,
        size: u64,
        file_hash: blake3::Hash,
        /// Bytes already present from an earlier attempt (received files only)
        resumed_bytes: u64,
        elapsed: Duration,
    },
    /// Over for good: refused, declined, changed file, failed integrity check
    Failed { error: String },
    /// The connection was lost; it can resume later
    Interrupted { error: String },
}

/// Result of a successful [`Node::send_file`]
#[derive(Debug, Clone)]
pub struct SendReport {
    pub transfer_id: TransferId,
    pub size: u64,
    pub file_hash: blake3::Hash,
    pub elapsed: Duration,
}

/// A running TOVI device. Cheap to clone; clones share the same device.
#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
}

struct Inner {
    identity: Arc<DeviceIdentity>,
    name: String,
    store: Arc<Store>,
    endpoint: QuicEndpoint,
    manager: Arc<PairingManager>,
    /// Replaced when the receive folder changes
    inbox: RwLock<Arc<Inbox>>,
    events: broadcast::Sender<Event>,
    pending: Mutex<HashMap<u64, oneshot::Sender<bool>>>,
    next_request: AtomicU64,
    auto_approve_pairing: bool,
    approval_timeout: Duration,
    /// Background work, so shutdown can wait for all of it
    accept_task: Mutex<Option<JoinHandle<()>>>,
    connection_tasks: Mutex<JoinSet<()>>,
}

impl Node {
    /// Load or create this device's identity and database, start listening,
    /// and return the node with a receiver for its events. Must be called
    /// inside a Tokio runtime.
    pub async fn start(config: NodeConfig) -> Result<(Node, broadcast::Receiver<Event>)> {
        let identity = Arc::new(DeviceIdentity::load_or_generate(&FileKeyStore::new(
            config.data_dir.join("identity.key"),
        ))?);
        let store = Arc::new(Store::open(&config.data_dir.join("tovi.db"))?);
        let trust: Arc<dyn TrustStore> = store.clone();
        let manager = Arc::new(PairingManager::new(identity.public_key(), trust));

        let endpoint = match QuicEndpoint::bind(identity.clone(), config.listen, manager.clone()) {
            Ok(endpoint) => endpoint,
            Err(e) if config.listen.port() != 0 => {
                tracing::warn!("{e:#}; listening on a free port instead");
                let any_port = SocketAddr::new(config.listen.ip(), 0);
                QuicEndpoint::bind(identity.clone(), any_port, manager.clone())?
            }
            Err(e) => return Err(e),
        };

        let (events, receiver) = broadcast::channel(EVENT_BUFFER);
        let receive_dir = config.receive_dir_override.clone().unwrap_or_else(|| {
            stored_receive_dir(&store).unwrap_or_else(|| config.default_receive_dir.clone())
        });
        let inbox = Arc::new(Inbox::with_history(receive_dir, store.clone()));
        let node = Node {
            inner: Arc::new(Inner {
                identity,
                name: config.device_name,
                store,
                endpoint,
                manager,
                inbox: RwLock::new(inbox),
                events,
                pending: Mutex::new(HashMap::new()),
                next_request: AtomicU64::new(1),
                auto_approve_pairing: config.auto_approve_pairing,
                approval_timeout: config.approval_timeout,
                accept_task: Mutex::new(None),
                connection_tasks: Mutex::new(JoinSet::new()),
            }),
        };
        let accept = tokio::spawn(accept_loop(node.inner.clone()));
        *node.inner.accept_task.lock().unwrap() = Some(accept);
        Ok((node, receiver))
    }

    /// Another receiver for this node's events
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    pub fn device_id(&self) -> DeviceId {
        self.inner.identity.device_id()
    }

    pub fn device_name(&self) -> &str {
        &self.inner.name
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.endpoint.local_addr()
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.inner.store
    }

    // ---------------------------------------------------------- pairing

    /// Open a pairing session reachable at this machine's network addresses;
    /// show the code's [`PairingCode::to_uri`] as a QR code
    pub fn new_pairing_code(&self) -> Result<PairingCode> {
        let addresses = candidate_addresses(self.local_addr()?.port())?;
        if addresses.is_empty() {
            bail!("no usable network address; connect to Wi-Fi or Ethernet");
        }
        Ok(self.new_pairing_code_with(addresses))
    }

    /// Open a pairing session advertising exactly `endpoints`
    pub fn new_pairing_code_with(&self, endpoints: Vec<SocketAddr>) -> PairingCode {
        self.inner.manager.start_session(endpoints)
    }

    /// Pair with the device whose code was scanned or pasted
    pub async fn pair(&self, uri: &str) -> Result<TrustedDevice> {
        let inner = &self.inner;
        let code = PairingCode::from_uri(uri.trim())?;
        let (conn, device) = pairing::initiate(
            &inner.endpoint,
            &code,
            &Hello::new(inner.name.clone()),
            &self.device_id(),
            inner.store.as_ref(),
        )
        .await?;
        // The address that answered goes first next time
        let mut addresses = vec![conn.remote_address()];
        addresses.extend(
            code.endpoints
                .iter()
                .filter(|a| **a != conn.remote_address()),
        );
        inner.store.set_device_addresses(&device.id, &addresses)?;
        conn.close();
        inner.emit(Event::Paired {
            device: device.clone(),
        });
        inner.emit(Event::DevicesChanged);
        Ok(device)
    }

    /// Answer a [`Event::PairingRequested`] or [`Event::IncomingOffer`].
    /// Returns false if the request already expired or was answered.
    pub fn respond(&self, request_id: u64, allow: bool) -> bool {
        let waiting = self.inner.pending.lock().unwrap().remove(&request_id);
        waiting.is_some_and(|tx| tx.send(allow).is_ok())
    }

    // ---------------------------------------------------------- devices

    pub fn devices(&self) -> Result<Vec<DeviceDetails>> {
        self.inner.store.device_details()
    }

    /// Find a paired device by name or ID prefix (see [`Store::find_device`])
    pub fn find_device(&self, query: &str) -> Result<Option<TrustedDevice>> {
        self.inner.store.find_device(query)
    }

    /// Stop trusting a device. Returns whether it was paired.
    pub fn forget(&self, id: &DeviceId) -> Result<bool> {
        let removed = self.inner.store.remove(id)?;
        if removed {
            self.inner.emit(Event::DevicesChanged);
        }
        Ok(removed)
    }

    pub fn history(&self, limit: usize) -> Result<Vec<TransferRecord>> {
        self.inner.store.history(limit)
    }

    // --------------------------------------------------------- settings

    pub fn receive_dir(&self) -> PathBuf {
        self.inner.inbox.read().unwrap().dir().to_path_buf()
    }

    /// Save a new receive folder; it applies to the next transfer. Partial
    /// files of interrupted transfers stay in the old folder.
    pub fn set_receive_dir(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        // Not canonicalize(): on Windows that yields verbatim `\\?\C:\...` paths
        let dir = std::path::absolute(dir)?;
        self.inner
            .store
            .set_setting(SETTING_RECEIVE_DIR, &dir.to_string_lossy())?;
        *self.inner.inbox.write().unwrap() =
            Arc::new(Inbox::with_history(dir, self.inner.store.clone()));
        Ok(())
    }

    /// Whether files from paired devices are accepted without asking (default: yes)
    pub fn auto_accept(&self) -> bool {
        self.inner.auto_accept()
    }

    pub fn set_auto_accept(&self, on: bool) -> Result<()> {
        self.inner
            .store
            .set_setting(SETTING_AUTO_ACCEPT, if on { "true" } else { "false" })
    }

    // ---------------------------------------------------------- sending

    /// Send a file to a paired device at its last known addresses. Reconnects
    /// and resumes after drops; an unfinished earlier send of the same file
    /// to the same device is resumed. Returns once the receiver has verified
    /// and saved it.
    pub async fn send_file(&self, to: &DeviceId, path: &Path) -> Result<SendReport> {
        let inner = &self.inner;
        let device = inner
            .store
            .get(to)?
            .context("that device is not paired; pair with it first")?;
        let addresses = inner.store.device_addresses(&device.id)?;
        if addresses.is_empty() {
            bail!(
                "no known address for {}; pair again with its code",
                device.name
            );
        }
        let key = device.id.public_key()?;

        let options = SendOptions::default();
        let fresh = OutgoingTransfer::new(path, options)?;
        let previous = inner.store.unfinished_send(
            &device.id,
            fresh.path(),
            fresh.file_size(),
            fresh.modified(),
        )?;
        let transfer = match previous {
            Some(id) => OutgoingTransfer::with_id(path, options, id)?,
            None => fresh,
        };
        let id = transfer.transfer_id();
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        inner.emit(Event::TransferStarted {
            transfer_id: id,
            direction: Direction::Sent,
            device: device.clone(),
            file_name: file_name.clone(),
            file_size: transfer.file_size(),
            resuming: previous.is_some(),
        });
        let finish = |outcome: TransferOutcome| {
            inner.emit(Event::TransferFinished {
                transfer_id: id,
                direction: Direction::Sent,
                device: device.clone(),
                file_name: file_name.clone(),
                outcome,
            })
        };

        let conn = match inner.endpoint.connect_any(&addresses, &key).await {
            Ok(conn) => conn,
            Err(e) => {
                let error = format!(
                    "could not reach {} at its last known address; is TOVI running there? ({e:#})",
                    device.name
                );
                finish(TransferOutcome::Interrupted {
                    error: error.clone(),
                });
                bail!(error);
            }
        };
        inner.store.touch_device(&device.id)?;
        inner
            .store
            .record_transfer_started(&transfer.history_record(&device.id))?;

        let started = Instant::now();
        let progress = ProgressThrottle::new(inner, id, Direction::Sent);
        let result = transfer::send_with_resume(
            &inner.endpoint,
            &addresses,
            &key,
            conn,
            &transfer,
            |p| progress.update(p),
            |e| {
                inner.emit(Event::TransferReconnecting {
                    transfer_id: id,
                    error: format!("{e:#}"),
                })
            },
        )
        .await;

        match result {
            Ok((conn, file_hash)) => {
                conn.close();
                inner
                    .store
                    .record_transfer_completed(&id, None, &file_hash)?;
                let report = SendReport {
                    transfer_id: id,
                    size: transfer.file_size(),
                    file_hash,
                    elapsed: started.elapsed(),
                };
                finish(TransferOutcome::Completed {
                    path: None,
                    size: report.size,
                    file_hash,
                    resumed_bytes: 0,
                    elapsed: report.elapsed,
                });
                Ok(report)
            }
            // A lost connection can resume later; anything else ends the transfer
            Err(e) if e.downcast_ref::<ConnectionLost>().is_some() => {
                finish(TransferOutcome::Interrupted {
                    error: format!("{e:#}"),
                });
                Err(e)
            }
            Err(e) => {
                inner.store.record_transfer_failed(&id, &format!("{e:#}"))?;
                finish(TransferOutcome::Failed {
                    error: format!("{e:#}"),
                });
                Err(e)
            }
        }
    }

    /// Stop accepting connections, close open ones, and wait until peers
    /// have been told. Returns once all background work has finished, so
    /// the database and files are closed when the last `Node` is dropped.
    pub async fn shutdown(&self) {
        self.inner.endpoint.shutdown().await;
        let accept = self.inner.accept_task.lock().unwrap().take();
        if let Some(accept) = accept {
            let _ = accept.await;
        }
        // The accept loop has ended, so no new connection tasks can appear
        let mut connections = std::mem::take(&mut *self.inner.connection_tasks.lock().unwrap());
        while connections.join_next().await.is_some() {}
    }
}

fn stored_receive_dir(store: &Store) -> Option<PathBuf> {
    store
        .setting(SETTING_RECEIVE_DIR)
        .ok()
        .flatten()
        .map(PathBuf::from)
}

impl Inner {
    fn emit(&self, event: Event) {
        // No subscribers is fine: nobody is listening right now
        let _ = self.events.send(event);
    }

    fn auto_accept(&self) -> bool {
        match self.store.setting(SETTING_AUTO_ACCEPT) {
            Ok(Some(value)) => value != "false",
            Ok(None) => true,
            Err(e) => {
                tracing::warn!("could not read auto-accept setting, asking instead: {e:#}");
                false
            }
        }
    }

    /// Ask the user (via an event) and wait for [`Node::respond`]; declines
    /// after the approval timeout
    async fn ask(&self, make_event: impl FnOnce(u64) -> Event) -> bool {
        let request_id = self.next_request.fetch_add(1, Ordering::Relaxed);
        let (answer, waiting) = oneshot::channel();
        self.pending.lock().unwrap().insert(request_id, answer);
        self.emit(make_event(request_id));

        let answer = tokio::time::timeout(self.approval_timeout, waiting).await;
        self.pending.lock().unwrap().remove(&request_id);
        match answer {
            Ok(Ok(allow)) => allow,
            Ok(Err(_)) => false,
            Err(_) => {
                self.emit(Event::RequestExpired { request_id });
                false
            }
        }
    }

    fn inbox(&self) -> Arc<Inbox> {
        self.inbox.read().unwrap().clone()
    }
}

async fn accept_loop(inner: Arc<Inner>) {
    while let Some(incoming) = inner.endpoint.accept().await {
        match incoming {
            Ok(conn) => {
                let mut tasks = inner.connection_tasks.lock().unwrap();
                // Forget connections that already ended, so the set stays small
                while tasks.try_join_next().is_some() {}
                tasks.spawn(handle_connection(inner.clone(), conn));
            }
            Err(e) => tracing::info!("refused a connection: {e:#}"),
        }
    }
}

/// Each stream is routed on its first message: `HELLO` starts pairing, a
/// `TRANSFER_OFFER` sends a file. Untrusted devices may only pair; trusted
/// ones may send files, and may also pair again (they scanned a new code,
/// e.g. after forgetting this device on their side).
async fn handle_connection(inner: Arc<Inner>, conn: PeerConnection) {
    let mut device = match inner.store.get(conn.peer_id()) {
        Ok(known) => known,
        Err(e) => {
            tracing::error!("could not read paired devices: {e:#}");
            conn.refuse();
            return;
        }
    };
    if let Some(device) = &device {
        remember_peer(&inner, &conn, device);
    }

    loop {
        let first = match conn.next_bi().await {
            Ok(Some((send, mut recv))) => protocol::read_message(&mut recv)
                .await
                .map(|message| (send, recv, message)),
            Ok(None) => return,
            Err(e) => Err(e),
        };
        let (send, recv, message) = match first {
            Ok(first) => first,
            Err(e) if device.is_none() => return pairing_failed(&inner, &conn, e),
            Err(e) => {
                tracing::warn!("receive from {} failed: {e:#}", conn.peer_id());
                if conn.is_closed() {
                    return;
                }
                continue;
            }
        };
        match message {
            Message::Hello(hello) => match pair_on(&inner, &conn, send, recv, hello).await {
                Some(paired) => device = Some(paired),
                None => return,
            },
            Message::TransferOffer(offer) => {
                let Some(device) = &device else {
                    // Not paired: shut the door so the device doesn't retry
                    let e = anyhow!("an unpaired device tried to send a file");
                    return pairing_failed(&inner, &conn, e);
                };
                if !receive_one(&inner, &conn, device, send, recv, offer).await {
                    return;
                }
            }
            other => {
                let e = anyhow!("expected HELLO or TRANSFER_OFFER, got {}", other.kind());
                if device.is_none() {
                    return pairing_failed(&inner, &conn, e);
                }
                tracing::warn!("receive from {} failed: {e:#}", conn.peer_id());
            }
        }
    }
}

/// Answer a `HELLO` with the pairing exchange; still needs a valid code and
/// the user's approval, also from a device that is already trusted. Returns
/// the (updated) trusted device, or `None` after refusing the connection.
async fn pair_on(
    inner: &Arc<Inner>,
    conn: &PeerConnection,
    send: SendStream,
    recv: RecvStream,
    their_hello: Hello,
) -> Option<TrustedDevice> {
    let hello = Hello::new(inner.name.clone());
    let ask = |request: PairingRequest| async move {
        if inner.auto_approve_pairing {
            return true;
        }
        inner
            .ask(|request_id| Event::PairingRequested {
                request_id,
                request,
            })
            .await
    };
    let result =
        pairing::respond_to_hello(conn, send, recv, their_hello, &inner.manager, &hello, ask).await;
    match result {
        Ok(device) => {
            remember_peer(inner, conn, &device);
            // Only now: once `Paired` is out, the app may send to this device at once
            inner.emit(Event::Paired {
                device: device.clone(),
            });
            inner.emit(Event::DevicesChanged);
            Some(device)
        }
        Err(e) => {
            pairing_failed(inner, conn, e);
            None
        }
    }
}

/// Not paired: shut the door so the device doesn't retry, and say why
fn pairing_failed(inner: &Inner, conn: &PeerConnection, error: anyhow::Error) {
    conn.refuse();
    inner.emit(Event::PairingFailed {
        device_id: *conn.peer_id(),
        error: format!("{error:#}"),
    });
}

/// Their endpoint also listens, so this is where to reach them next time
fn remember_peer(inner: &Inner, conn: &PeerConnection, device: &TrustedDevice) {
    if let Err(e) = inner.store.touch_device(&device.id).and_then(|()| {
        inner
            .store
            .remember_address(&device.id, conn.remote_address())
    }) {
        tracing::warn!("could not update {}: {e:#}", device.name);
    }
}

/// Receive the file `offer` describes, on the stream it arrived on. Returns
/// whether to wait for another.
async fn receive_one(
    inner: &Arc<Inner>,
    conn: &PeerConnection,
    device: &TrustedDevice,
    send: SendStream,
    recv: RecvStream,
    offer: TransferOffer,
) -> bool {
    let started = Instant::now();
    // Filled in once the offer arrives, so later events can name the transfer
    let current: Mutex<Option<(TransferId, String)>> = Mutex::new(None);
    let progress: Mutex<Option<ProgressThrottle>> = Mutex::new(None);

    let (current_ref, progress_ref) = (&current, &progress);
    let approve = move |offer: TransferOffer| async move {
        let id = offer.transfer_id;
        *current_ref.lock().unwrap() = Some((id, offer.file_name.clone()));
        let allow = inner.auto_accept()
            || inner
                .ask(|request_id| Event::IncomingOffer {
                    request_id,
                    transfer_id: id,
                    from: device.clone(),
                    file_name: offer.file_name.clone(),
                    file_size: offer.file_size,
                })
                .await;
        if allow {
            *progress_ref.lock().unwrap() =
                Some(ProgressThrottle::new(inner, id, Direction::Received));
            inner.emit(Event::TransferStarted {
                transfer_id: id,
                direction: Direction::Received,
                device: device.clone(),
                file_name: offer.file_name,
                file_size: offer.file_size,
                resuming: false,
            });
        }
        allow
    };
    let on_progress = |p: Progress| {
        if let Some(throttle) = progress.lock().unwrap().as_ref() {
            throttle.update(p);
        }
    };

    let result = inner
        .inbox()
        .receive_offer(conn, send, recv, offer, approve, on_progress)
        .await;

    let (outcome, keep_going, id, file_name) = match result {
        Ok(file) => {
            let name = file
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let outcome = TransferOutcome::Completed {
                path: Some(file.path),
                size: file.size,
                file_hash: file.file_hash,
                resumed_bytes: file.resumed_bytes,
                elapsed: started.elapsed(),
            };
            (outcome, true, file.transfer_id, name)
        }
        Err(e) => {
            let Some((id, name)) = current.lock().unwrap().clone() else {
                // Refused before the offer was understood (e.g. no disk space)
                tracing::warn!("receive from {} failed: {e:#}", device.name);
                return !conn.is_closed();
            };
            let error = format!("{e:#}");
            let outcome = if conn.is_closed() {
                TransferOutcome::Interrupted { error }
            } else {
                TransferOutcome::Failed { error }
            };
            (outcome, false, id, name)
        }
    };
    inner.emit(Event::TransferFinished {
        transfer_id: id,
        direction: Direction::Received,
        device: device.clone(),
        file_name,
        outcome,
    });
    keep_going
}

/// Turns progress callbacks into at most ~10 events a second, with speed
struct ProgressThrottle {
    events: broadcast::Sender<Event>,
    transfer_id: TransferId,
    direction: Direction,
    state: Mutex<ThrottleState>,
}

struct ThrottleState {
    first: Option<(Instant, u64)>,
    last_emit: Option<Instant>,
}

impl ProgressThrottle {
    fn new(inner: &Inner, transfer_id: TransferId, direction: Direction) -> Self {
        Self {
            events: inner.events.clone(),
            transfer_id,
            direction,
            state: Mutex::new(ThrottleState {
                first: None,
                last_emit: None,
            }),
        }
    }

    fn update(&self, p: Progress) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap();
        let (start, start_done) = *state.first.get_or_insert((now, p.done));
        let due = state
            .last_emit
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        if !due && p.done < p.total {
            return;
        }
        state.last_emit = Some(now);
        let secs = now.duration_since(start).as_secs_f64();
        let bytes_per_sec = if secs > 0.0 {
            ((p.done - start_done) as f64 / secs) as u64
        } else {
            0
        };
        let _ = self.events.send(Event::TransferProgress {
            transfer_id: self.transfer_id,
            direction: self.direction,
            done: p.done,
            total: p.total,
            bytes_per_sec,
        });
    }
}

#[cfg(test)]
mod tests;
