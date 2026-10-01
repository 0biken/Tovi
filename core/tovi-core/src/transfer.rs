//! Chunked, resumable, hash-verified transfer engine (decisions.md D5, D6).
//!
//! Sender ([`OutgoingTransfer`]): offers the file on a new bidirectional
//! stream. The receiver answers with the chunks it already holds (when
//! resuming). The sender then reads the whole file once, in order, hashing
//! each chunk and the whole file with BLAKE3, and sends the chunks the
//! receiver lacks, up to [`PARALLEL_CHUNKS`] at a time, each on its own
//! unidirectional stream. Then it sends the whole-file hash.
//!
//! Receiver ([`Inbox`]): sanitises the name, checks free space and asks for
//! approval; writes each chunk into `<name>.<id>.tovi.part` only after its
//! hash matches, recording progress in `<name>.<id>.tovi.state`; then re-reads
//! the finished file and compares the whole-file hash (catching disk write
//! errors too) before renaming it to a free name.
//!
//! Resume (TASKS §8): if the connection drops, the part and state files are
//! kept. The sender reconnects and offers the same transfer ID; the receiver
//! finds the state file and the sender skips the chunks already held.
//! [`send_with_resume`] does the reconnecting. Any other failure (bad chunk,
//! decline, protocol error) deletes both files.
//!
//! One transfer at a time per connection.

use crate::identity::DeviceId;
use crate::protocol::{
    self, ChunkHeader, Message, TransferComplete, TransferOffer, TransferResponse, TransferResult,
    MAX_HAVE_RANGES,
};
use crate::transport::{PeerConnection, QuicEndpoint};
use anyhow::{anyhow, bail, ensure, Context, Result};
use ed25519_dalek::VerifyingKey;
use quinn::{RecvStream, SendStream};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::io::AsyncReadExt;
use tokio::sync::watch;
use tokio::task::JoinSet;

/// Default chunk size (decisions.md D5, provisional until benchmarked)
pub const DEFAULT_CHUNK_SIZE: u32 = 8 * 1024 * 1024;
pub const MIN_CHUNK_SIZE: u32 = 64 * 1024;
pub const MAX_CHUNK_SIZE: u32 = 16 * 1024 * 1024;
/// Chunks in flight at once, per transfer
pub const PARALLEL_CHUNKS: usize = 4;
/// How long [`send_with_resume`] keeps trying to reconnect after a drop
pub const RECONNECT_WINDOW: Duration = Duration::from_secs(120);

const PART_SUFFIX: &str = ".tovi.part";
const STATE_SUFFIX: &str = ".tovi.state";
const STATE_VERSION: u8 = 1;
/// Free space to leave on the receiving disk beyond the file itself
const DISK_SPACE_MARGIN: u64 = 64 * 1024 * 1024;
/// Longest sanitised file name in bytes; leaves room for " (n)" and the part
/// suffix within the common 255-byte limit
const MAX_FILE_NAME_BYTES: usize = 200;

pub type TransferId = [u8; 16];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct SendOptions {
    pub chunk_size: u32,
}

impl Default for SendOptions {
    fn default() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReceivedFile {
    pub path: PathBuf,
    pub size: u64,
    pub file_hash: blake3::Hash,
    /// Bytes that were already on disk from an earlier, interrupted attempt
    pub resumed_bytes: u64,
}

/// The file changed on the sender since the transfer started, so it can't be
/// resumed; start a new transfer instead
#[derive(Debug)]
pub struct SourceChanged(pub PathBuf);

impl fmt::Display for SourceChanged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} changed since the transfer started", self.0.display())
    }
}

impl std::error::Error for SourceChanged {}

fn chunk_count(file_size: u64, chunk_size: u32) -> u64 {
    file_size.div_ceil(u64::from(chunk_size))
}

fn chunk_len(file_size: u64, chunk_size: u32, index: u64) -> usize {
    let offset = index * u64::from(chunk_size);
    // At most chunk_size, which fits in usize
    (file_size - offset).min(u64::from(chunk_size)) as usize
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Received-chunk bitmap ⇄ `[start, end)` ranges
fn to_ranges(received: &[bool]) -> Vec<[u64; 2]> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (i, &have) in received.iter().chain([false].iter()).enumerate() {
        match (have, start) {
            (true, None) => start = Some(i as u64),
            (false, Some(s)) => {
                ranges.push([s, i as u64]);
                start = None;
            }
            _ => {}
        }
    }
    ranges
}

fn from_ranges(ranges: &[[u64; 2]], count: u64) -> Result<Vec<bool>> {
    let mut received = vec![false; count as usize];
    for &[start, end] in ranges {
        ensure!(
            start < end && end <= count,
            "chunk range {start}..{end} out of bounds"
        );
        received[start as usize..end as usize].fill(true);
    }
    Ok(received)
}

// ---------------------------------------------------------------- sending

/// A file being sent. Keep it across reconnects: re-sending it reuses its
/// transfer ID, so the receiver can resume.
#[derive(Debug, Clone)]
pub struct OutgoingTransfer {
    transfer_id: TransferId,
    path: PathBuf,
    file_name: String,
    file_size: u64,
    modified: Option<SystemTime>,
    chunk_size: u32,
}

impl OutgoingTransfer {
    pub fn new(path: &Path, options: SendOptions) -> Result<Self> {
        let chunk_size = options.chunk_size;
        ensure!(
            (MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size),
            "chunk size {chunk_size} out of range"
        );
        let metadata = fs::metadata(path).with_context(|| format!("opening {}", path.display()))?;
        ensure!(metadata.is_file(), "{} is not a file", path.display());
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow!("{} has no usable file name", path.display()))?
            .to_string();
        let mut transfer_id = [0u8; 16];
        OsRng.fill_bytes(&mut transfer_id);
        Ok(Self {
            transfer_id,
            path: path.to_path_buf(),
            file_name,
            file_size: metadata.len(),
            modified: metadata.modified().ok(),
            chunk_size,
        })
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    /// One attempt: offer (or resume) the file, send what the receiver lacks,
    /// and wait until it has verified and saved the file
    pub async fn send(
        &self,
        conn: &PeerConnection,
        progress: impl Fn(Progress),
    ) -> Result<blake3::Hash> {
        let metadata = fs::metadata(&self.path)?;
        if metadata.len() != self.file_size || metadata.modified().ok() != self.modified {
            return Err(SourceChanged(self.path.clone()).into());
        }
        let mut file = tokio::fs::File::open(&self.path).await?;

        let id = self.transfer_id;
        let (mut control_send, mut control_recv) = conn.open_bi().await?;
        let offer = TransferOffer {
            transfer_id: id,
            file_name: self.file_name.clone(),
            file_size: self.file_size,
            chunk_size: self.chunk_size,
        };
        protocol::write_message(&mut control_send, &Message::TransferOffer(offer)).await?;
        let count = chunk_count(self.file_size, self.chunk_size);
        let have = match protocol::read_message(&mut control_recv).await? {
            Message::TransferResponse(r) if r.transfer_id == id => {
                if !r.accepted {
                    bail!("transfer declined: {}", r.reason.unwrap_or_default());
                }
                from_ranges(&r.have, count)?
            }
            other => bail!("expected TRANSFER_RESPONSE, got {other:?}"),
        };

        let total = self.file_size;
        let mut done: u64 = (0..count)
            .filter(|&i| have[i as usize])
            .map(|i| chunk_len(self.file_size, self.chunk_size, i) as u64)
            .sum();
        progress(Progress { done, total });
        let mut file_hasher = blake3::Hasher::new();
        let mut uploads = JoinSet::new();

        // Read every chunk (the whole-file hash needs them all), send the missing ones
        for index in 0..count {
            let mut data = vec![0u8; chunk_len(self.file_size, self.chunk_size, index)];
            file.read_exact(&mut data)
                .await
                .map_err(|_| SourceChanged(self.path.clone()))?;
            file_hasher.update(&data);
            if have[index as usize] {
                continue;
            }
            let header = ChunkHeader {
                transfer_id: id,
                index,
                hash: *blake3::hash(&data).as_bytes(),
            };
            if uploads.len() >= PARALLEL_CHUNKS {
                done += join_upload(&mut uploads).await?;
                progress(Progress { done, total });
            }
            let conn = conn.clone();
            uploads.spawn(async move { send_chunk(&conn, &header, &data).await });
        }
        while !uploads.is_empty() {
            done += join_upload(&mut uploads).await?;
            progress(Progress { done, total });
        }

        let file_hash = file_hasher.finalize();
        let complete = TransferComplete {
            transfer_id: id,
            file_hash: *file_hash.as_bytes(),
        };
        protocol::write_message(&mut control_send, &Message::TransferComplete(complete)).await?;
        control_send.finish()?;

        match protocol::read_message(&mut control_recv).await? {
            Message::TransferResult(r) if r.transfer_id == id => {
                if !r.ok {
                    bail!(
                        "receiver rejected the file: {}",
                        r.reason.unwrap_or_default()
                    );
                }
            }
            other => bail!("expected TRANSFER_RESULT, got {other:?}"),
        }
        Ok(file_hash)
    }
}

/// Send the file at `path` in one attempt, without resuming
pub async fn send_file(
    conn: &PeerConnection,
    path: &Path,
    options: SendOptions,
    progress: impl Fn(Progress),
) -> Result<blake3::Hash> {
    OutgoingTransfer::new(path, options)?
        .send(conn, progress)
        .await
}

/// Send `transfer`, reconnecting and resuming if the connection drops.
///
/// Reconnects to `addrs` (pinning `server_key`) for up to
/// [`RECONNECT_WINDOW`] after each drop; `on_drop` is told about each one.
/// Returns the connection in use at the end, which may be a new one.
pub async fn send_with_resume(
    endpoint: &QuicEndpoint,
    addrs: &[SocketAddr],
    server_key: &VerifyingKey,
    mut conn: PeerConnection,
    transfer: &OutgoingTransfer,
    progress: impl Fn(Progress),
    on_drop: impl Fn(&anyhow::Error),
) -> Result<(PeerConnection, blake3::Hash)> {
    loop {
        match transfer.send(&conn, &progress).await {
            Ok(hash) => return Ok((conn, hash)),
            // Only a lost connection is worth retrying; refusals and integrity
            // failures arrive on a connection that is still open
            Err(e) if conn.is_closed() => {
                on_drop(&e);
                conn = reconnect(endpoint, addrs, server_key)
                    .await
                    .with_context(|| format!("connection lost ({e:#}) and could not reconnect"))?;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn reconnect(
    endpoint: &QuicEndpoint,
    addrs: &[SocketAddr],
    server_key: &VerifyingKey,
) -> Result<PeerConnection> {
    let deadline = Instant::now() + RECONNECT_WINDOW;
    let mut delay = Duration::from_millis(500);
    loop {
        match endpoint.connect_any(addrs, server_key).await {
            Ok(conn) => return Ok(conn),
            Err(e) if Instant::now() + delay >= deadline => return Err(e),
            Err(_) => {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(5));
            }
        }
    }
}

async fn join_upload(uploads: &mut JoinSet<Result<u64>>) -> Result<u64> {
    uploads
        .join_next()
        .await
        .expect("join_upload called with no uploads")?
}

async fn send_chunk(conn: &PeerConnection, header: &ChunkHeader, data: &[u8]) -> Result<u64> {
    let mut stream = conn.open_uni().await?;
    protocol::write_message(&mut stream, &Message::Chunk(header.clone())).await?;
    stream.write_all(data).await?;
    stream.finish()?;
    Ok(data.len() as u64)
}

// -------------------------------------------------------------- receiving

/// Where incoming files are saved, and which transfers are in progress.
///
/// Share one `Inbox` across all connections: when a sender reconnects after
/// a drop, the new connection takes over (supersedes) the transfer from the
/// old one, which may not have noticed the drop yet.
pub struct Inbox {
    dir: PathBuf,
    active: Mutex<HashMap<TransferId, ActiveTransfer>>,
    next_generation: AtomicU64,
}

struct ActiveTransfer {
    generation: u64,
    cancel: watch::Sender<bool>,
    slot: Arc<tokio::sync::Mutex<()>>,
}

/// Persisted receive progress: the `.tovi.state` file
#[derive(Serialize, Deserialize)]
struct ReceiveState {
    v: u8,
    #[serde(with = "serde_bytes")]
    transfer_id: TransferId,
    #[serde(with = "serde_bytes")]
    sender: [u8; 32],
    file_name: String,
    file_size: u64,
    chunk_size: u32,
    received: Vec<[u64; 2]>,
}

struct Partial {
    part_path: PathBuf,
    state_path: PathBuf,
    state: ReceiveState,
    received: Vec<bool>,
    file: Arc<Mutex<File>>,
}

impl Partial {
    /// Pick up an earlier attempt's files, if they belong to this offer and
    /// sender; otherwise start fresh
    fn open(dir: &Path, name: &str, offer: &TransferOffer, sender: &DeviceId) -> Result<Self> {
        let stem = format!("{name}.{}", hex(&offer.transfer_id[..4]));
        let part_path = dir.join(format!("{stem}{PART_SUFFIX}"));
        let state_path = dir.join(format!("{stem}{STATE_SUFFIX}"));
        let count = chunk_count(offer.file_size, offer.chunk_size);

        if let Some(state) = Self::load_state(&state_path) {
            let matches = state.v == STATE_VERSION
                && state.transfer_id == offer.transfer_id
                && state.file_size == offer.file_size
                && state.chunk_size == offer.chunk_size
                && state.file_name == name;
            ensure!(
                !matches || state.sender == *sender.as_bytes(),
                "transfer ID already in use by another device"
            );
            let file = OpenOptions::new().read(true).write(true).open(&part_path);
            if let (true, Ok(file)) = (matches, file) {
                if file.metadata()?.len() == offer.file_size {
                    let received = from_ranges(&state.received, count)?;
                    return Ok(Self {
                        part_path,
                        state_path,
                        state,
                        received,
                        file: Arc::new(Mutex::new(file)),
                    });
                }
            }
            // Stale or unusable leftovers under this name: discard them
            let _ = fs::remove_file(&part_path);
            let _ = fs::remove_file(&state_path);
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&part_path)
            .with_context(|| format!("creating {}", part_path.display()))?;
        file.set_len(offer.file_size)?;
        let partial = Self {
            part_path,
            state_path,
            state: ReceiveState {
                v: STATE_VERSION,
                transfer_id: offer.transfer_id,
                sender: *sender.as_bytes(),
                file_name: name.to_string(),
                file_size: offer.file_size,
                chunk_size: offer.chunk_size,
                received: Vec::new(),
            },
            received: vec![false; count as usize],
            file: Arc::new(Mutex::new(file)),
        };
        partial.save()?;
        Ok(partial)
    }

    fn load_state(path: &Path) -> Option<ReceiveState> {
        let bytes = fs::read(path).ok()?;
        ciborium::from_reader(bytes.as_slice()).ok()
    }

    fn received_bytes(&self) -> u64 {
        self.received
            .iter()
            .enumerate()
            .filter(|(_, &have)| have)
            .map(|(i, _)| chunk_len(self.state.file_size, self.state.chunk_size, i as u64) as u64)
            .sum()
    }

    /// Record a verified, written chunk. The state file is replaced
    /// atomically, so a crash leaves either the old or the new version.
    fn record(&mut self, index: u64) -> Result<()> {
        let seen = &mut self.received[index as usize];
        ensure!(!*seen, "chunk {index} sent twice");
        *seen = true;
        self.save()
    }

    fn save(&self) -> Result<()> {
        let mut state = Vec::new();
        let snapshot = ReceiveState {
            received: to_ranges(&self.received),
            file_name: self.state.file_name.clone(),
            ..self.state
        };
        ciborium::into_writer(&snapshot, &mut state)?;
        let tmp = self.state_path.with_extension("state.tmp");
        fs::write(&tmp, state)?;
        fs::rename(&tmp, &self.state_path)?;
        Ok(())
    }

    fn discard(&self) {
        let _ = fs::remove_file(&self.part_path);
        let _ = fs::remove_file(&self.state_path);
    }
}

impl Inbox {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            active: Mutex::new(HashMap::new()),
            next_generation: AtomicU64::new(0),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Wait for the peer's next file and receive (or resume) it.
    ///
    /// `approve` sees the offer and decides whether to accept it. Returns
    /// `Ok(None)` when the peer closes the connection instead of sending.
    pub async fn receive_next<F, Fut>(
        &self,
        conn: &PeerConnection,
        approve: F,
        progress: impl Fn(Progress),
    ) -> Result<Option<ReceivedFile>>
    where
        F: FnOnce(TransferOffer) -> Fut,
        Fut: Future<Output = bool>,
    {
        let Some((mut control_send, mut control_recv)) = conn.next_bi().await? else {
            return Ok(None);
        };
        let offer = match protocol::read_message(&mut control_recv).await? {
            Message::TransferOffer(offer) => offer,
            other => bail!("expected TRANSFER_OFFER, got {other:?}"),
        };
        let id = offer.transfer_id;

        if let Err(e) = check_offer(&offer, &self.dir) {
            refuse(&mut control_send, id, &e.to_string()).await?;
            return Err(e.context("refused transfer offer"));
        }
        if !approve(offer.clone()).await {
            refuse(&mut control_send, id, "declined on the other device").await?;
            bail!("declined {}", offer.file_name);
        }

        // Take over the transfer from any earlier connection, then wait for it to let go
        let (generation, mut superseded, slot) = self.claim(id);
        let slot_guard = slot.lock_owned().await;

        let name = sanitize_file_name(&offer.file_name);
        let mut partial = match Partial::open(&self.dir, &name, &offer, conn.peer_id()) {
            Ok(partial) => partial,
            Err(e) => {
                drop(slot_guard);
                self.release(id, generation);
                refuse(&mut control_send, id, &e.to_string()).await?;
                return Err(e);
            }
        };
        let resumed_bytes = partial.received_bytes();
        let accept = Message::TransferResponse(TransferResponse {
            transfer_id: id,
            accepted: true,
            reason: None,
            have: to_ranges(&partial.received)
                .into_iter()
                .take(MAX_HAVE_RANGES)
                .collect(),
        });
        protocol::write_message(&mut control_send, &accept).await?;

        let body = receive_body(conn, &offer, &mut partial, &mut control_recv, progress);
        let (received, was_superseded) = tokio::select! {
            result = body => (result, false),
            _ = superseded.wait_for(|cancelled| *cancelled) => {
                (Err(anyhow!("superseded by a newer connection for the same transfer")), true)
            }
        };

        let outcome = received.and_then(|file_hash| {
            let path = finalize(&partial.part_path, &self.dir, &name)?;
            let _ = fs::remove_file(&partial.state_path);
            Ok(ReceivedFile {
                path,
                size: offer.file_size,
                file_hash,
                resumed_bytes,
            })
        });
        if outcome.is_err() && !was_superseded && !conn.is_closed() {
            // Not a dropped connection: this attempt is over for good
            partial.discard();
        }
        drop(partial);
        drop(slot_guard);
        self.release(id, generation);

        let reason = outcome.as_ref().err().map(|e| format!("{e:#}"));
        let result = TransferResult {
            transfer_id: id,
            ok: reason.is_none(),
            reason: reason.map(|r| r.chars().take(200).collect()),
        };
        // Best effort: the sender may already be gone
        let _ = protocol::finish_with(&mut control_send, &Message::TransferResult(result)).await;
        outcome.map(Some)
    }

    /// Register `id` as active on a new connection, signalling any earlier
    /// connection handling it to stop
    fn claim(&self, id: TransferId) -> (u64, watch::Receiver<bool>, Arc<tokio::sync::Mutex<()>>) {
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let (cancel, cancelled) = watch::channel(false);
        let mut active = self.active.lock().unwrap();
        let slot = match active.get_mut(&id) {
            Some(existing) => {
                let _ = existing.cancel.send(true);
                existing.cancel = cancel;
                existing.generation = generation;
                existing.slot.clone()
            }
            None => {
                let slot = Arc::new(tokio::sync::Mutex::new(()));
                active.insert(
                    id,
                    ActiveTransfer {
                        generation,
                        cancel,
                        slot: slot.clone(),
                    },
                );
                slot
            }
        };
        (generation, cancelled, slot)
    }

    fn release(&self, id: TransferId, generation: u64) {
        let mut active = self.active.lock().unwrap();
        if active.get(&id).is_some_and(|a| a.generation == generation) {
            active.remove(&id);
        }
    }
}

fn check_offer(offer: &TransferOffer, receive_dir: &Path) -> Result<()> {
    ensure!(
        (MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&offer.chunk_size),
        "unsupported chunk size {}",
        offer.chunk_size
    );
    fs::create_dir_all(receive_dir)
        .with_context(|| format!("creating {}", receive_dir.display()))?;
    let available = fs4::available_space(receive_dir)?;
    ensure!(
        offer.file_size.saturating_add(DISK_SPACE_MARGIN) <= available,
        "not enough disk space ({} MB needed, {} MB free)",
        offer.file_size / 1_000_000,
        available / 1_000_000
    );
    Ok(())
}

async fn refuse(send: &mut SendStream, id: TransferId, reason: &str) -> Result<()> {
    let response = Message::TransferResponse(TransferResponse {
        transfer_id: id,
        accepted: false,
        reason: Some(reason.chars().take(200).collect()),
        have: Vec::new(),
    });
    protocol::finish_with(send, &response).await
}

/// Receive the missing chunks into the part file, then check the whole-file hash
async fn receive_body(
    conn: &PeerConnection,
    offer: &TransferOffer,
    partial: &mut Partial,
    control_recv: &mut RecvStream,
    progress: impl Fn(Progress),
) -> Result<blake3::Hash> {
    let missing = partial.received.iter().filter(|&&have| !have).count();
    let total = offer.file_size;
    let mut done = partial.received_bytes();
    progress(Progress { done, total });

    let mut tasks = JoinSet::new();
    let mut record = |partial: &mut Partial, result: Result<(u64, usize)>| -> Result<()> {
        let (index, len) = result?;
        partial.record(index)?;
        done += len as u64;
        progress(Progress { done, total });
        Ok(())
    };
    // Accept new chunk streams and record finished chunks as they happen, so
    // the state file is current even if the sender stalls mid-transfer
    let mut accepted = 0;
    while accepted < missing || !tasks.is_empty() {
        tokio::select! {
            Some(result) = tasks.join_next(), if !tasks.is_empty() => record(partial, result?)?,
            stream = conn.accept_uni(), if accepted < missing && tasks.len() < PARALLEL_CHUNKS => {
                tasks.spawn(receive_chunk(stream?, offer.clone(), partial.file.clone()));
                accepted += 1;
            }
        }
    }

    let claimed = match protocol::read_message(control_recv).await? {
        Message::TransferComplete(c) if c.transfer_id == offer.transfer_id => {
            blake3::Hash::from_bytes(c.file_hash)
        }
        other => bail!("expected TRANSFER_COMPLETE, got {other:?}"),
    };

    // Re-read what is actually on disk, so write errors are caught too
    let file = partial.file.clone();
    let actual = tokio::task::spawn_blocking(move || -> Result<blake3::Hash> {
        let mut file = file.lock().unwrap();
        file.sync_all()?;
        file.seek(SeekFrom::Start(0))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update_reader(&mut *file)?;
        Ok(hasher.finalize())
    })
    .await??;
    ensure!(actual == claimed, "file failed its integrity check");
    Ok(actual)
}

/// Read one chunk stream, verify its hash, and write it at its offset
async fn receive_chunk(
    mut stream: RecvStream,
    offer: TransferOffer,
    file: Arc<Mutex<File>>,
) -> Result<(u64, usize)> {
    let header = match protocol::read_message(&mut stream).await? {
        Message::Chunk(header) => header,
        other => bail!("expected CHUNK, got {other:?}"),
    };
    ensure!(
        header.transfer_id == offer.transfer_id,
        "chunk from another transfer"
    );
    let count = chunk_count(offer.file_size, offer.chunk_size);
    ensure!(
        header.index < count,
        "chunk index {} out of range",
        header.index
    );

    let len = chunk_len(offer.file_size, offer.chunk_size, header.index);
    let mut data = vec![0u8; len];
    stream.read_exact(&mut data).await?;
    // blake3::Hash equality is constant-time
    ensure!(
        blake3::hash(&data) == blake3::Hash::from_bytes(header.hash),
        "chunk {} failed its integrity check",
        header.index
    );

    let offset = header.index * u64::from(offer.chunk_size);
    tokio::task::spawn_blocking(move || -> Result<()> {
        let mut file = file.lock().unwrap();
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&data)?;
        Ok(())
    })
    .await??;
    Ok((header.index, len))
}

/// Move the verified part file to a free name in `dir`.
///
/// Rename replaces existing files on every platform, so the free name is
/// checked first; another program creating the same name in between could
/// still be overwritten.
fn finalize(part_path: &Path, dir: &Path, name: &str) -> Result<PathBuf> {
    let destination = unique_destination(dir, name)?;
    fs::rename(part_path, &destination)
        .with_context(|| format!("saving {}", destination.display()))?;
    Ok(destination)
}

fn unique_destination(dir: &Path, name: &str) -> Result<PathBuf> {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return Ok(candidate);
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    for n in 1..1000 {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!("too many files named {name}")
}

/// Turn a name from another device into a safe single file name.
///
/// Applies Windows rules everywhere (the strictest), so a name behaves the
/// same on every receiving platform: no directories, no reserved device names,
/// no characters Windows forbids, no trailing dots or spaces, bounded length.
pub fn sanitize_file_name(raw: &str) -> String {
    // Keep only the last path component, whichever separator the sender used
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut name: String = base
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = name.trim_end_matches(['.', ' ']).len();
    name.truncate(trimmed);
    if name.trim().is_empty() {
        name = "file".into();
    }

    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        name.insert(0, '_');
    }

    if name.len() > MAX_FILE_NAME_BYTES {
        let ext = match name.rfind('.') {
            Some(dot) if name.len() - dot <= 16 => name[dot..].to_string(),
            _ => String::new(),
        };
        let mut cut = MAX_FILE_NAME_BYTES - ext.len();
        while !name.is_char_boundary(cut) {
            cut -= 1;
        }
        name = format!("{}{ext}", &name[..cut]);
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;
    use std::net::{IpAddr, Ipv4Addr};

    const LOCALHOST: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

    /// Two endpoints and one connection between them. The endpoints must stay
    /// alive with the connections; `receiver_endpoint` can accept reconnects.
    struct Link {
        sender: PeerConnection,
        receiver: PeerConnection,
        sender_endpoint: QuicEndpoint,
        receiver_endpoint: QuicEndpoint,
        receiver_key: VerifyingKey,
    }

    async fn link() -> Link {
        let allow_all = || Arc::new(|_: &DeviceId| true);
        let a = Arc::new(DeviceIdentity::generate_new());
        let b = Arc::new(DeviceIdentity::generate_new());
        let sender_endpoint = QuicEndpoint::bind(a, LOCALHOST, allow_all()).unwrap();
        let receiver_endpoint = QuicEndpoint::bind(b.clone(), LOCALHOST, allow_all()).unwrap();
        let addr = receiver_endpoint.local_addr().unwrap();
        let receiver_key = b.public_key();
        let (sender, receiver) = tokio::join!(
            sender_endpoint.connect(addr, &receiver_key),
            receiver_endpoint.accept()
        );
        Link {
            sender: sender.unwrap(),
            receiver: receiver.unwrap().unwrap(),
            sender_endpoint,
            receiver_endpoint,
            receiver_key,
        }
    }

    fn temp_dir() -> PathBuf {
        let mut id = [0u8; 8];
        OsRng.fill_bytes(&mut id);
        let dir = std::env::temp_dir().join(format!("tovi-transfer-test-{}", hex(&id)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn random_bytes(len: usize) -> Vec<u8> {
        let mut data = vec![0u8; len];
        OsRng.fill_bytes(&mut data);
        data
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn spawn_receive(
        inbox: Arc<Inbox>,
        conn: PeerConnection,
        allow: bool,
    ) -> tokio::task::JoinHandle<Result<Option<ReceivedFile>>> {
        tokio::spawn(async move {
            inbox
                .receive_next(&conn, |_| async move { allow }, |_| {})
                .await
        })
    }

    /// Send `data` as `name` and receive it, auto-approving
    async fn round_trip(name: &str, data: &[u8], chunk_size: u32, inbox: &Path) -> ReceivedFile {
        let link = link().await;
        let outbox = temp_dir();
        let source = outbox.join(name);
        fs::write(&source, data).unwrap();

        let receiving = spawn_receive(Arc::new(Inbox::new(inbox)), link.receiver.clone(), true);
        let last_progress = Mutex::new(None);
        let sent_hash = send_file(&link.sender, &source, SendOptions { chunk_size }, |p| {
            *last_progress.lock().unwrap() = Some(p)
        })
        .await
        .unwrap();
        let received = receiving.await.unwrap().unwrap().unwrap();

        assert_eq!(received.file_hash, sent_hash);
        assert_eq!(received.resumed_bytes, 0);
        assert_eq!(
            *last_progress.lock().unwrap(),
            Some(Progress {
                done: data.len() as u64,
                total: data.len() as u64
            })
        );
        fs::remove_dir_all(outbox).unwrap();
        received
    }

    #[tokio::test]
    async fn file_arrives_intact_across_many_chunks() {
        let inbox = temp_dir();
        let data = random_bytes(300 * 1024 + 17); // 5 chunks, last one partial
        let received = round_trip("photo.jpg", &data, MIN_CHUNK_SIZE, &inbox).await;

        assert_eq!(received.path, inbox.join("photo.jpg"));
        assert_eq!(fs::read(&received.path).unwrap(), data);
        assert_eq!(files_in(&inbox), vec!["photo.jpg"]); // no part or state file left
        fs::remove_dir_all(inbox).unwrap();
    }

    #[tokio::test]
    async fn default_chunk_size_handles_multi_chunk_file() {
        let inbox = temp_dir();
        let data = random_bytes(DEFAULT_CHUNK_SIZE as usize * 2 + 1234);
        let received = round_trip("video.mp4", &data, DEFAULT_CHUNK_SIZE, &inbox).await;

        assert_eq!(fs::read(&received.path).unwrap(), data);
        fs::remove_dir_all(inbox).unwrap();
    }

    #[tokio::test]
    async fn empty_file_transfers() {
        let inbox = temp_dir();
        let received = round_trip("empty.txt", &[], MIN_CHUNK_SIZE, &inbox).await;

        assert_eq!(received.size, 0);
        assert_eq!(fs::read(&received.path).unwrap(), b"");
        assert_eq!(files_in(&inbox), vec!["empty.txt"]);
        fs::remove_dir_all(inbox).unwrap();
    }

    #[tokio::test]
    async fn duplicate_names_get_a_number() {
        let inbox = temp_dir();
        fs::write(inbox.join("notes.txt"), b"already here").unwrap();
        let received = round_trip("notes.txt", b"new notes", MIN_CHUNK_SIZE, &inbox).await;

        assert_eq!(received.path, inbox.join("notes (1).txt"));
        assert_eq!(fs::read(inbox.join("notes.txt")).unwrap(), b"already here");
        fs::remove_dir_all(inbox).unwrap();
    }

    #[tokio::test]
    async fn declined_transfer_leaves_nothing() {
        let link = link().await;
        let inbox = temp_dir();
        let outbox = temp_dir();
        let source = outbox.join("secret.pdf");
        fs::write(&source, b"data").unwrap();

        let receiving = spawn_receive(Arc::new(Inbox::new(&inbox)), link.receiver.clone(), false);
        let err = send_file(&link.sender, &source, SendOptions::default(), |_| {})
            .await
            .unwrap_err();

        assert!(err.to_string().contains("declined"), "{err}");
        assert!(receiving.await.unwrap().is_err());
        assert!(files_in(&inbox).is_empty());
        fs::remove_dir_all(inbox).unwrap();
        fs::remove_dir_all(outbox).unwrap();
    }

    /// Hand-rolled sender: offer `size` bytes as transfer `id`, return the control streams
    async fn offer_raw(
        conn: &PeerConnection,
        id: TransferId,
        name: &str,
        size: u64,
        chunk_size: u32,
    ) -> (SendStream, RecvStream, TransferResponse) {
        let (mut control, mut control_recv) = conn.open_bi().await.unwrap();
        let offer = TransferOffer {
            transfer_id: id,
            file_name: name.into(),
            file_size: size,
            chunk_size,
        };
        protocol::write_message(&mut control, &Message::TransferOffer(offer))
            .await
            .unwrap();
        let response = match protocol::read_message(&mut control_recv).await.unwrap() {
            Message::TransferResponse(r) => r,
            other => panic!("expected TRANSFER_RESPONSE, got {other:?}"),
        };
        (control, control_recv, response)
    }

    #[tokio::test]
    async fn corrupted_chunk_is_rejected_and_partial_files_removed() {
        let link = link().await;
        let inbox = temp_dir();
        let receiving = spawn_receive(Arc::new(Inbox::new(&inbox)), link.receiver.clone(), true);

        let id = [3u8; 16];
        let (_control, mut control_recv, response) =
            offer_raw(&link.sender, id, "evil.bin", 10, MIN_CHUNK_SIZE).await;
        assert!(response.accepted);
        let header = ChunkHeader {
            transfer_id: id,
            index: 0,
            hash: *blake3::hash(b"0123456789").as_bytes(),
        };
        send_chunk(&link.sender, &header, b"0123456XXX")
            .await
            .unwrap();

        match protocol::read_message(&mut control_recv).await.unwrap() {
            Message::TransferResult(r) => {
                assert!(!r.ok);
                assert!(r.reason.unwrap().contains("integrity"));
            }
            other => panic!("expected TRANSFER_RESULT, got {other:?}"),
        }
        drop(control_recv);
        let err = receiving.await.unwrap().unwrap_err();
        assert!(err.to_string().contains("integrity"), "{err:#}");
        assert!(files_in(&inbox).is_empty(), "{:?}", files_in(&inbox));
        fs::remove_dir_all(inbox).unwrap();
    }

    #[tokio::test]
    async fn offer_larger_than_free_space_is_refused() {
        let link = link().await;
        let inbox = temp_dir();
        let receiving = spawn_receive(Arc::new(Inbox::new(&inbox)), link.receiver.clone(), true);

        let (_control, control_recv, response) = offer_raw(
            &link.sender,
            [4; 16],
            "huge.iso",
            u64::MAX / 2,
            MAX_CHUNK_SIZE,
        )
        .await;
        assert!(!response.accepted);
        assert!(response.reason.unwrap().contains("disk space"));

        drop(control_recv);
        assert!(receiving.await.unwrap().is_err());
        assert!(files_in(&inbox).is_empty());
        fs::remove_dir_all(inbox).unwrap();
    }

    #[tokio::test]
    async fn closed_connection_ends_receiving_cleanly() {
        let link = link().await;
        link.sender.close();
        let inbox = temp_dir();
        let result = Inbox::new(&inbox)
            .receive_next(&link.receiver, |_| async { true }, |_| {})
            .await;
        assert!(result.unwrap().is_none());
        fs::remove_dir_all(inbox).unwrap();
    }

    #[tokio::test]
    async fn transfer_resumes_after_connection_drop() {
        let link = link().await;
        let inbox = temp_dir();
        let outbox = temp_dir();
        let data = random_bytes(MIN_CHUNK_SIZE as usize * 20 + 99); // 21 chunks
        let source = outbox.join("holiday.mov");
        fs::write(&source, &data).unwrap();

        // First attempt: the receiver drops the connection after a few chunks
        let dropper = link.receiver.clone();
        let first_inbox = Arc::new(Inbox::new(&inbox));
        let first = {
            let conn = link.receiver.clone();
            let inbox = first_inbox.clone();
            tokio::spawn(async move {
                inbox
                    .receive_next(
                        &conn,
                        |_| async { true },
                        |p| {
                            if p.done >= u64::from(MIN_CHUNK_SIZE) * 5 {
                                dropper.close();
                            }
                        },
                    )
                    .await
            })
        };

        // Second attempt arrives on a new connection, handled by a fresh Inbox,
        // as if the receiving app had restarted
        let addr = link.receiver_endpoint.local_addr().unwrap();
        let receiver_endpoint = link.receiver_endpoint;
        let second_inbox = inbox.clone();
        let second = tokio::spawn(async move {
            let conn = receiver_endpoint.accept().await.unwrap().unwrap();
            let resumed_at = Mutex::new(None);
            let result = Inbox::new(&second_inbox)
                .receive_next(
                    &conn,
                    |_| async { true },
                    |p| {
                        resumed_at.lock().unwrap().get_or_insert(p.done);
                    },
                )
                .await;
            let resumed_at = *resumed_at.lock().unwrap();
            (result, resumed_at, receiver_endpoint)
        });

        let transfer = OutgoingTransfer::new(
            &source,
            SendOptions {
                chunk_size: MIN_CHUNK_SIZE,
            },
        )
        .unwrap();
        let drops = AtomicU64::new(0);
        let (_conn, hash) = send_with_resume(
            &link.sender_endpoint,
            &[addr],
            &link.receiver_key,
            link.sender.clone(),
            &transfer,
            |_| {},
            |_| {
                drops.fetch_add(1, Ordering::Relaxed);
            },
        )
        .await
        .unwrap();

        assert!(
            first.await.unwrap().is_err(),
            "first attempt should end with the drop"
        );
        let (result, resumed_at, _endpoint) = second.await.unwrap();
        let received = result.unwrap().unwrap();
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(received.file_hash, hash);
        assert!(received.resumed_bytes >= u64::from(MIN_CHUNK_SIZE) * 5);
        assert_eq!(resumed_at, Some(received.resumed_bytes));
        assert_eq!(fs::read(&received.path).unwrap(), data);
        assert_eq!(files_in(&inbox), vec!["holiday.mov"]);
        fs::remove_dir_all(inbox).unwrap();
        fs::remove_dir_all(outbox).unwrap();
    }

    #[tokio::test]
    async fn new_connection_takes_over_a_stalled_transfer() {
        let link = link().await;
        let inbox = Arc::new(Inbox::new(temp_dir()));
        let outbox = temp_dir();
        let data = random_bytes(MIN_CHUNK_SIZE as usize * 3);
        let source = outbox.join("report.pdf");
        fs::write(&source, &data).unwrap();
        let mut transfer = OutgoingTransfer::new(
            &source,
            SendOptions {
                chunk_size: MIN_CHUNK_SIZE,
            },
        )
        .unwrap();
        let id = transfer.transfer_id;

        // A first connection sends chunk 0, then goes silent without closing
        let got_first_chunk = Arc::new(tokio::sync::Notify::new());
        let stalled = {
            let inbox = inbox.clone();
            let conn = link.receiver.clone();
            let signal = got_first_chunk.clone();
            tokio::spawn(async move {
                inbox
                    .receive_next(
                        &conn,
                        |_| async { true },
                        |p| {
                            if p.done > 0 {
                                signal.notify_one();
                            }
                        },
                    )
                    .await
            })
        };
        let (_control, _control_recv, response) = offer_raw(
            &link.sender,
            id,
            "report.pdf",
            data.len() as u64,
            MIN_CHUNK_SIZE,
        )
        .await;
        assert!(response.accepted && response.have.is_empty());
        let header = ChunkHeader {
            transfer_id: id,
            index: 0,
            hash: *blake3::hash(&data[..MIN_CHUNK_SIZE as usize]).as_bytes(),
        };
        send_chunk(&link.sender, &header, &data[..MIN_CHUNK_SIZE as usize])
            .await
            .unwrap();
        got_first_chunk.notified().await;

        // The sender reconnects; the stalled connection is still open
        let addr = link.receiver_endpoint.local_addr().unwrap();
        let (new_sender, new_receiver) = tokio::join!(
            link.sender_endpoint.connect(addr, &link.receiver_key),
            link.receiver_endpoint.accept()
        );
        let resumed = spawn_receive(inbox.clone(), new_receiver.unwrap().unwrap(), true);
        transfer.transfer_id = id; // same transfer, resumed
        let hash = transfer.send(&new_sender.unwrap(), |_| {}).await.unwrap();

        let received = resumed.await.unwrap().unwrap().unwrap();
        assert_eq!(received.file_hash, hash);
        assert_eq!(received.resumed_bytes, u64::from(MIN_CHUNK_SIZE));
        assert_eq!(fs::read(&received.path).unwrap(), data);
        let err = stalled.await.unwrap().unwrap_err();
        assert!(err.to_string().contains("superseded"), "{err:#}");
        assert_eq!(files_in(inbox.dir()), vec!["report.pdf"]);
        fs::remove_dir_all(inbox.dir()).unwrap();
        fs::remove_dir_all(outbox).unwrap();
    }

    #[tokio::test]
    async fn changed_source_file_is_not_resumed() {
        let link = link().await;
        let outbox = temp_dir();
        let source = outbox.join("draft.txt");
        fs::write(&source, b"version one").unwrap();
        let transfer = OutgoingTransfer::new(&source, SendOptions::default()).unwrap();

        fs::write(&source, b"version two, longer").unwrap();
        let err = transfer.send(&link.sender, |_| {}).await.unwrap_err();
        assert!(err.downcast_ref::<SourceChanged>().is_some(), "{err:#}");
        fs::remove_dir_all(outbox).unwrap();
    }

    #[tokio::test]
    async fn fully_received_file_completes_on_resume() {
        // A drop after the last chunk but before the final hash: nothing left to send
        let link = link().await;
        let inbox = temp_dir();
        let outbox = temp_dir();
        let data = random_bytes(MIN_CHUNK_SIZE as usize * 2);
        let source = outbox.join("done.bin");
        fs::write(&source, &data).unwrap();
        let transfer = OutgoingTransfer::new(
            &source,
            SendOptions {
                chunk_size: MIN_CHUNK_SIZE,
            },
        )
        .unwrap();

        // Leave behind exactly what a dropped receiver would: full part file + state
        let name = "done.bin";
        let offer = TransferOffer {
            transfer_id: transfer.transfer_id,
            file_name: name.into(),
            file_size: data.len() as u64,
            chunk_size: MIN_CHUNK_SIZE,
        };
        let sender_id = *link.receiver.peer_id();
        let mut partial = Partial::open(&inbox, name, &offer, &sender_id).unwrap();
        partial.file.lock().unwrap().write_all(&data).unwrap();
        partial.record(0).unwrap();
        partial.record(1).unwrap();
        drop(partial);

        let receiving = spawn_receive(Arc::new(Inbox::new(&inbox)), link.receiver.clone(), true);
        let first_progress = Mutex::new(None);
        transfer
            .send(&link.sender, |p| {
                first_progress.lock().unwrap().get_or_insert(p);
            })
            .await
            .unwrap();
        let received = receiving.await.unwrap().unwrap().unwrap();

        assert_eq!(received.resumed_bytes, data.len() as u64);
        assert_eq!(
            first_progress.lock().unwrap().unwrap().done,
            data.len() as u64
        );
        assert_eq!(fs::read(&received.path).unwrap(), data);
        assert_eq!(files_in(&inbox), vec!["done.bin"]);
        fs::remove_dir_all(inbox).unwrap();
        fs::remove_dir_all(outbox).unwrap();
    }

    #[test]
    fn ranges_round_trip() {
        let received = [true, true, false, true, false, false, true];
        let ranges = to_ranges(&received);
        assert_eq!(ranges, vec![[0, 2], [3, 4], [6, 7]]);
        assert_eq!(from_ranges(&ranges, 7).unwrap(), received);
        assert!(to_ranges(&[false, false]).is_empty());
        assert!(from_ranges(&[[2, 9]], 7).is_err());
        assert!(from_ranges(&[[3, 3]], 7).is_err());
    }

    #[test]
    fn sanitize_strips_paths_and_unsafe_characters() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(
            sanitize_file_name(r"C:\Windows\System32\evil.dll"),
            "evil.dll"
        );
        assert_eq!(
            sanitize_file_name("a<b>c:d\"e|f?g*.txt"),
            "a_b_c_d_e_f_g_.txt"
        );
        assert_eq!(sanitize_file_name("tab\there.txt"), "tab_here.txt");
        assert_eq!(sanitize_file_name("report.pdf. . "), "report.pdf");
    }

    #[test]
    fn sanitize_never_returns_an_empty_or_dot_name() {
        for raw in ["", ".", "..", "...", "   ", "dir/", "dir\\"] {
            assert_eq!(sanitize_file_name(raw), "file", "input {raw:?}");
        }
        assert_eq!(sanitize_file_name(".bashrc"), ".bashrc");
    }

    #[test]
    fn sanitize_escapes_windows_device_names() {
        assert_eq!(sanitize_file_name("CON"), "_CON");
        assert_eq!(sanitize_file_name("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_file_name("COM1.log"), "_COM1.log");
        assert_eq!(sanitize_file_name("LPT9"), "_LPT9");
        assert_eq!(sanitize_file_name("CONSOLE.txt"), "CONSOLE.txt");
        assert_eq!(sanitize_file_name("COM10"), "COM10");
    }

    #[test]
    fn sanitize_limits_length_and_keeps_extension() {
        let long = format!("{}.jpeg", "é".repeat(300));
        let name = sanitize_file_name(&long);
        assert!(name.len() <= MAX_FILE_NAME_BYTES);
        assert!(name.ends_with(".jpeg"));
    }

    #[test]
    fn chunk_arithmetic() {
        assert_eq!(chunk_count(0, MIN_CHUNK_SIZE), 0);
        assert_eq!(chunk_count(1, MIN_CHUNK_SIZE), 1);
        assert_eq!(chunk_count(u64::from(MIN_CHUNK_SIZE), MIN_CHUNK_SIZE), 1);
        assert_eq!(
            chunk_count(u64::from(MIN_CHUNK_SIZE) + 1, MIN_CHUNK_SIZE),
            2
        );
        assert_eq!(chunk_len(100_000, MIN_CHUNK_SIZE, 1), 100_000 - 65_536);
    }
}
