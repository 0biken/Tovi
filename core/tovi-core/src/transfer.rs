//! Chunked, resumable, hash-verified transfer engine (decisions.md D5, D6).
//!
//! Sender: offers the file on a new bidirectional stream; once accepted, reads
//! the file once, in order, hashing each chunk and the whole file with BLAKE3,
//! and sends up to [`PARALLEL_CHUNKS`] chunks at a time, each on its own
//! unidirectional stream. Then it sends the whole-file hash.
//!
//! Receiver: sanitises the name, checks free space and asks for approval;
//! writes each chunk into `<name>.<id>.tovi.part` only after its hash matches;
//! then re-reads the finished file and compares it with the sender's
//! whole-file hash (which also catches disk write errors) before renaming it
//! to its final, non-clashing name. On any failure the part file is deleted.
//!
//! One transfer at a time per connection. Resume (TASKS §8) is not built yet.

use crate::protocol::{
    self, ChunkHeader, Message, TransferComplete, TransferOffer, TransferResponse, TransferResult,
};
use crate::transport::PeerConnection;
use anyhow::{anyhow, bail, ensure, Context, Result};
use quinn::{RecvStream, SendStream};
use rand_core::{OsRng, RngCore};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::future::Future;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncReadExt;
use tokio::task::JoinSet;

/// Default chunk size (decisions.md D5, provisional until benchmarked)
pub const DEFAULT_CHUNK_SIZE: u32 = 8 * 1024 * 1024;
pub const MIN_CHUNK_SIZE: u32 = 64 * 1024;
pub const MAX_CHUNK_SIZE: u32 = 16 * 1024 * 1024;
/// Chunks in flight at once, per transfer
pub const PARALLEL_CHUNKS: usize = 4;

const PART_SUFFIX: &str = ".tovi.part";
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
}

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

// ---------------------------------------------------------------- sending

/// Send the file at `path` to the peer. Returns once the peer has verified
/// and saved it.
pub async fn send_file(
    conn: &PeerConnection,
    path: &Path,
    options: SendOptions,
    progress: impl Fn(Progress),
) -> Result<blake3::Hash> {
    let chunk_size = options.chunk_size;
    ensure!(
        (MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size),
        "chunk size {chunk_size} out of range"
    );
    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("opening {}", path.display()))?;
    let file_size = file.metadata().await?.len();
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("{} has no usable file name", path.display()))?
        .to_string();

    let mut transfer_id = [0u8; 16];
    OsRng.fill_bytes(&mut transfer_id);
    let (mut control_send, mut control_recv) = conn.open_bi().await?;
    let offer = TransferOffer {
        transfer_id,
        file_name,
        file_size,
        chunk_size,
    };
    protocol::write_message(&mut control_send, &Message::TransferOffer(offer)).await?;
    match protocol::read_message(&mut control_recv).await? {
        Message::TransferResponse(r) if r.transfer_id == transfer_id => {
            if !r.accepted {
                bail!("transfer declined: {}", r.reason.unwrap_or_default());
            }
        }
        other => bail!("expected TRANSFER_RESPONSE, got {other:?}"),
    }

    let total = file_size;
    let mut done = 0u64;
    progress(Progress { done, total });
    let mut file_hasher = blake3::Hasher::new();
    let mut uploads = JoinSet::new();

    for index in 0..chunk_count(file_size, chunk_size) {
        let mut data = vec![0u8; chunk_len(file_size, chunk_size, index)];
        file.read_exact(&mut data)
            .await
            .context("file changed size while sending")?;
        file_hasher.update(&data);
        let header = ChunkHeader {
            transfer_id,
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
        transfer_id,
        file_hash: *file_hash.as_bytes(),
    };
    protocol::write_message(&mut control_send, &Message::TransferComplete(complete)).await?;
    control_send.finish()?;

    match protocol::read_message(&mut control_recv).await? {
        Message::TransferResult(r) if r.transfer_id == transfer_id => {
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

/// Wait for the peer's next file and receive it into `receive_dir`.
///
/// `approve` sees the offer and decides whether to accept it. Returns
/// `Ok(None)` when the peer closes the connection instead of sending.
pub async fn receive_next<F, Fut>(
    conn: &PeerConnection,
    receive_dir: &Path,
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

    if let Err(e) = check_offer(&offer, receive_dir) {
        respond(&mut control_send, id, Some(&e.to_string())).await?;
        return Err(e.context("refused transfer offer"));
    }
    if !approve(offer.clone()).await {
        respond(&mut control_send, id, Some("declined on the other device")).await?;
        bail!("declined {}", offer.file_name);
    }
    respond(&mut control_send, id, None).await?;

    let name = sanitize_file_name(&offer.file_name);
    let part_path = receive_dir.join(format!("{name}.{}{PART_SUFFIX}", hex(&id[..4])));
    let received = receive_body(conn, &offer, &part_path, &mut control_recv, progress).await;

    let outcome = received.and_then(|file_hash| {
        let path = finalize(&part_path, receive_dir, &name)?;
        Ok(ReceivedFile {
            path,
            size: offer.file_size,
            file_hash,
        })
    });
    let reason = outcome.as_ref().err().map(|e| format!("{e:#}"));
    if outcome.is_err() {
        let _ = fs::remove_file(&part_path);
    }
    let result = TransferResult {
        transfer_id: id,
        ok: reason.is_none(),
        reason: reason.map(|r| r.chars().take(200).collect()),
    };
    // Best effort: the sender may already be gone
    let _ = protocol::finish_with(&mut control_send, &Message::TransferResult(result)).await;
    outcome.map(Some)
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

async fn respond(send: &mut SendStream, id: TransferId, refusal: Option<&str>) -> Result<()> {
    let response = Message::TransferResponse(TransferResponse {
        transfer_id: id,
        accepted: refusal.is_none(),
        reason: refusal.map(Into::into),
    });
    if refusal.is_some() {
        protocol::finish_with(send, &response).await
    } else {
        protocol::write_message(send, &response).await
    }
}

/// Receive every chunk into the part file, then check the whole-file hash
async fn receive_body(
    conn: &PeerConnection,
    offer: &TransferOffer,
    part_path: &Path,
    control_recv: &mut RecvStream,
    progress: impl Fn(Progress),
) -> Result<blake3::Hash> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(part_path)
        .with_context(|| format!("creating {}", part_path.display()))?;
    file.set_len(offer.file_size)?;
    let file = Arc::new(Mutex::new(file));

    let count = chunk_count(offer.file_size, offer.chunk_size);
    let mut received = vec![false; count as usize];
    let total = offer.file_size;
    let mut done = 0u64;
    progress(Progress { done, total });

    let mut tasks = JoinSet::new();
    let mut record = |result: Result<(u64, usize)>| -> Result<()> {
        let (index, len) = result?;
        let seen = &mut received[index as usize];
        ensure!(!*seen, "chunk {index} sent twice");
        *seen = true;
        done += len as u64;
        progress(Progress { done, total });
        Ok(())
    };
    for _ in 0..count {
        let stream = conn.accept_uni().await?;
        if tasks.len() >= PARALLEL_CHUNKS {
            record(tasks.join_next().await.expect("tasks not empty")?)?;
        }
        tasks.spawn(receive_chunk(stream, offer.clone(), file.clone()));
    }
    while let Some(result) = tasks.join_next().await {
        record(result?)?;
    }

    let claimed = match protocol::read_message(control_recv).await? {
        Message::TransferComplete(c) if c.transfer_id == offer.transfer_id => {
            blake3::Hash::from_bytes(c.file_hash)
        }
        other => bail!("expected TRANSFER_COMPLETE, got {other:?}"),
    };

    // Re-read what is actually on disk, so write errors are caught too
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
    use crate::identity::{DeviceId, DeviceIdentity};
    use crate::transport::QuicEndpoint;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    const LOCALHOST: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

    /// Two connected endpoints; the endpoints must stay alive with the connections
    struct Link {
        sender: PeerConnection,
        receiver: PeerConnection,
        _endpoints: (QuicEndpoint, QuicEndpoint),
    }

    async fn link() -> Link {
        let allow_all = || Arc::new(|_: &DeviceId| true);
        let a = Arc::new(DeviceIdentity::generate_new());
        let b = Arc::new(DeviceIdentity::generate_new());
        let ea = QuicEndpoint::bind(a, LOCALHOST, allow_all()).unwrap();
        let eb = QuicEndpoint::bind(b.clone(), LOCALHOST, allow_all()).unwrap();
        let addr = eb.local_addr().unwrap();
        let b_key = b.public_key();
        let (sender, receiver) = tokio::join!(ea.connect(addr, &b_key), eb.accept());
        Link {
            sender: sender.unwrap(),
            receiver: receiver.unwrap().unwrap(),
            _endpoints: (ea, eb),
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

    /// Send `data` as `name` and receive it, auto-approving
    async fn round_trip(name: &str, data: &[u8], chunk_size: u32, inbox: &Path) -> ReceivedFile {
        let link = link().await;
        let outbox = temp_dir();
        let source = outbox.join(name);
        fs::write(&source, data).unwrap();

        let receiver = link.receiver.clone();
        let inbox_owned = inbox.to_path_buf();
        let receiving = tokio::spawn(async move {
            receive_next(&receiver, &inbox_owned, |_| async { true }, |_| {}).await
        });
        let last_progress = Mutex::new(None);
        let sent_hash = send_file(&link.sender, &source, SendOptions { chunk_size }, |p| {
            *last_progress.lock().unwrap() = Some(p)
        })
        .await
        .unwrap();
        let received = receiving.await.unwrap().unwrap().unwrap();

        assert_eq!(received.file_hash, sent_hash);
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
        assert_eq!(files_in(&inbox), vec!["photo.jpg"]); // no part file left behind
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

        let receiver = link.receiver.clone();
        let inbox_owned = inbox.clone();
        let receiving = tokio::spawn(async move {
            receive_next(&receiver, &inbox_owned, |_| async { false }, |_| {}).await
        });
        let err = send_file(&link.sender, &source, SendOptions::default(), |_| {})
            .await
            .unwrap_err();

        assert!(err.to_string().contains("declined"), "{err}");
        assert!(receiving.await.unwrap().is_err());
        assert!(files_in(&inbox).is_empty());
        fs::remove_dir_all(inbox).unwrap();
        fs::remove_dir_all(outbox).unwrap();
    }

    #[tokio::test]
    async fn corrupted_chunk_is_rejected_and_part_file_removed() {
        let link = link().await;
        let inbox = temp_dir();
        let receiver = link.receiver.clone();
        let inbox_owned = inbox.clone();
        let receiving = tokio::spawn(async move {
            receive_next(&receiver, &inbox_owned, |_| async { true }, |_| {}).await
        });

        // A hand-rolled sender whose chunk does not match its claimed hash
        let id = [3u8; 16];
        let (mut control, mut control_recv) = link.sender.open_bi().await.unwrap();
        let offer = TransferOffer {
            transfer_id: id,
            file_name: "evil.bin".into(),
            file_size: 10,
            chunk_size: MIN_CHUNK_SIZE,
        };
        protocol::write_message(&mut control, &Message::TransferOffer(offer))
            .await
            .unwrap();
        protocol::read_message(&mut control_recv).await.unwrap();
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
        assert!(files_in(&inbox).is_empty());
        fs::remove_dir_all(inbox).unwrap();
    }

    #[tokio::test]
    async fn offer_larger_than_free_space_is_refused() {
        let link = link().await;
        let inbox = temp_dir();
        let receiver = link.receiver.clone();
        let inbox_owned = inbox.clone();
        let receiving = tokio::spawn(async move {
            receive_next(&receiver, &inbox_owned, |_| async { true }, |_| {}).await
        });

        let (mut control, mut control_recv) = link.sender.open_bi().await.unwrap();
        let offer = TransferOffer {
            transfer_id: [4; 16],
            file_name: "huge.iso".into(),
            file_size: u64::MAX / 2,
            chunk_size: MAX_CHUNK_SIZE,
        };
        protocol::write_message(&mut control, &Message::TransferOffer(offer))
            .await
            .unwrap();
        match protocol::read_message(&mut control_recv).await.unwrap() {
            Message::TransferResponse(r) => {
                assert!(!r.accepted);
                assert!(r.reason.unwrap().contains("disk space"));
            }
            other => panic!("expected TRANSFER_RESPONSE, got {other:?}"),
        }
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
        let result = receive_next(&link.receiver, &inbox, |_| async { true }, |_| {}).await;
        assert!(result.unwrap().is_none());
        fs::remove_dir_all(inbox).unwrap();
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
