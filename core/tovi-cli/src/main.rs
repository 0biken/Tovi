//! Headless TOVI spike: pair two machines and send files from the terminal.
//!
//! Machine A (desktop role):  tovi-cli listen
//! Machine B (phone role):    tovi-cli send <file> "tovi://pair/..."   (first time)
//!                            tovi-cli send <file> <device name>       (afterwards)

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tovi_core::identity::{DeviceId, DeviceIdentity, FileKeyStore};
use tovi_core::pairing::{self, PairingCode, PairingManager, PairingRequest, CODE_LIFETIME};
use tovi_core::protocol::{Hello, MAX_DEVICE_NAME_LEN};
use tovi_core::storage::{Direction, Store, TransferStatus};
use tovi_core::transfer::{self, ConnectionLost, Inbox, OutgoingTransfer, Progress, SendOptions};
use tovi_core::transport::{candidate_addresses, PeerConnection, QuicEndpoint};
use tovi_core::trust::{TrustStore, TrustedDevice};

/// Default UDP port for `listen` (Tech doc §13)
const DEFAULT_PORT: u16 = 48210;

#[derive(Parser)]
#[command(
    name = "tovi-cli",
    about = "TOVI headless spike: pair devices and send files from the terminal"
)]
struct Cli {
    /// Where this device's identity and database are kept [default: OS data dir/TOVI]
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    /// Name shown to other devices [default: this computer's name]
    #[arg(long, global = true)]
    name: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show this device's ID and the addresses others can reach it at
    Id,
    /// Wait for devices and receive files, showing a QR code to pair a new one (desktop role)
    Listen {
        /// UDP port to listen on
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        /// Where received files are saved [default: Downloads/TOVI]
        #[arg(long)]
        receive_dir: Option<PathBuf>,
        /// Accept pairing requests without asking. For automated testing only.
        #[arg(long)]
        auto_approve: bool,
    },
    /// Pair with a device using the link from its QR code (phone role)
    Pair {
        /// The `tovi://pair/...` link
        uri: String,
    },
    /// Send a file to a paired device, or pair first using a QR link
    Send {
        /// File to send
        file: PathBuf,
        /// A paired device's name or ID prefix, or a `tovi://pair/...` link
        to: String,
    },
    /// List paired devices
    Devices,
    /// Stop trusting a paired device
    Forget {
        /// The device's name or ID prefix
        device: String,
    },
    /// Show recent transfers
    History {
        /// How many to show
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

/// This device: identity, name and local database
struct Me {
    identity: Arc<DeviceIdentity>,
    name: String,
    store: Arc<Store>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let data_dir = match cli.data_dir {
        Some(dir) => dir,
        None => dirs::data_dir()
            .context("no OS data directory; pass --data-dir")?
            .join("TOVI"),
    };
    let me = Me {
        identity: Arc::new(DeviceIdentity::load_or_generate(&FileKeyStore::new(
            data_dir.join("identity.key"),
        ))?),
        name: cli.name.unwrap_or_else(default_device_name),
        store: Arc::new(Store::open(&data_dir.join("tovi.db"))?),
    };

    match cli.command {
        Command::Id => show_id(&me, &data_dir),
        Command::Listen {
            port,
            receive_dir,
            auto_approve,
        } => {
            let receive_dir = match receive_dir {
                Some(dir) => dir,
                None => dirs::download_dir()
                    .context("no Downloads folder; pass --receive-dir")?
                    .join("TOVI"),
            };
            listen(me, port, receive_dir, auto_approve).await
        }
        Command::Pair { uri } => {
            let (endpoint, link) = pair(&me, &uri).await?;
            link.conn.close();
            endpoint.shutdown().await;
            Ok(())
        }
        Command::Send { file, to } => send(&me, &file, &to).await,
        Command::Devices => show_devices(&me.store),
        Command::Forget { device } => forget(&me.store, &device),
        Command::History { limit } => show_history(&me.store, limit),
    }
}

fn default_device_name() -> String {
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "TOVI device".into());
    name.chars().take(MAX_DEVICE_NAME_LEN).collect()
}

fn show_id(me: &Me, data_dir: &Path) -> Result<()> {
    println!("Name:       {}", me.name);
    println!("Device ID:  {}", me.identity.device_id());
    println!(
        "Data:       {} (identity key file is unencrypted in development builds)",
        data_dir.display()
    );
    let addresses = candidate_addresses(DEFAULT_PORT)?;
    if addresses.is_empty() {
        println!("Addresses:  none usable; connect to Wi-Fi or Ethernet");
    }
    for addr in addresses {
        println!("Address:    {}", addr.ip());
    }
    Ok(())
}

// -------------------------------------------------------------- listen

struct Listener {
    manager: Arc<PairingManager>,
    store: Arc<Store>,
    hello: Hello,
    /// Shared by all connections, so a reconnecting sender can resume
    inbox: Inbox,
    auto_approve: bool,
}

async fn listen(me: Me, port: u16, receive_dir: PathBuf, auto_approve: bool) -> Result<()> {
    let trust: Arc<dyn TrustStore> = me.store.clone();
    let manager = Arc::new(PairingManager::new(me.identity.public_key(), trust));
    let endpoint = QuicEndpoint::bind(
        me.identity.clone(),
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
        manager.clone(),
    )?;
    let port = endpoint.local_addr()?.port();

    let addresses = candidate_addresses(port)?;
    if addresses.is_empty() {
        bail!("no usable network address; connect to Wi-Fi or Ethernet");
    }
    let code = manager.start_session(addresses);
    print_pairing_code(&code)?;
    if auto_approve {
        println!("WARNING: --auto-approve is on; any device with this code will be trusted.\n");
    }
    let paired = me.store.list()?.len();
    println!(
        "{} is listening on port {port}. {paired} paired device(s) can send without a code.",
        me.name
    );
    println!(
        "Received files go to {}. Press Ctrl+C to stop.",
        receive_dir.display()
    );

    let listener = Arc::new(Listener {
        manager,
        store: me.store.clone(),
        hello: Hello::new(me.name),
        inbox: Inbox::with_history(receive_dir, me.store),
        auto_approve,
    });
    loop {
        let incoming = tokio::select! {
            incoming = endpoint.accept() => incoming,
            _ = tokio::signal::ctrl_c() => break,
        };
        let Some(result) = incoming else { break };
        match result {
            Ok(conn) => {
                tokio::spawn(handle_connection(listener.clone(), conn));
            }
            Err(e) => println!("Refused a connection: {e:#}"),
        }
    }

    println!("Shutting down...");
    endpoint.shutdown().await;
    Ok(())
}

/// Untrusted devices may only pair; trusted ones may send files
async fn handle_connection(listener: Arc<Listener>, conn: PeerConnection) {
    let known = match listener.store.get(conn.peer_id()) {
        Ok(known) => known,
        Err(e) => {
            println!("Could not read paired devices: {e:#}");
            return;
        }
    };
    let name = match known {
        Some(device) => {
            let _ = listener.store.touch_device(&device.id);
            device.name
        }
        None => {
            let ask = |req| ask_user(req, listener.auto_approve);
            match pairing::respond(&conn, &listener.manager, &listener.hello, ask).await {
                Ok(device) => {
                    println!("Paired with {} (saved)", describe(&device));
                    device.name
                }
                Err(e) => {
                    // Not paired: shut the door so the device doesn't retry
                    conn.refuse();
                    println!("Refused {}: not paired ({e:#})", short(conn.peer_id()));
                    return;
                }
            }
        }
    };

    let name = &name;
    loop {
        let progress = ProgressPrinter::new("Receiving");
        let received = listener
            .inbox
            .receive_next(
                &conn,
                |offer| async move {
                    println!(
                        "{name} is sending {} ({}). Accepted: paired device.",
                        offer.file_name,
                        megabytes(offer.file_size)
                    );
                    true
                },
                |p| progress.update(p),
            )
            .await;
        match received {
            Ok(Some(file)) => {
                let resumed = match file.resumed_bytes {
                    0 => String::new(),
                    bytes if bytes == file.size => ", already received earlier".into(),
                    bytes => format!(", resumed with {} already here", megabytes(bytes)),
                };
                println!(
                    "Saved {} ({}, {}{resumed})",
                    file.path.display(),
                    megabytes(file.size),
                    progress.rate(file.size - file.resumed_bytes)
                );
            }
            Ok(None) => break,
            Err(e) if conn.is_closed() => {
                println!(
                    "Connection to {name} lost ({e:#}); kept the partial file so it can resume"
                );
                break;
            }
            Err(e) => {
                println!("Receive failed: {e:#}");
                break;
            }
        }
    }
}

fn print_pairing_code(code: &PairingCode) -> Result<()> {
    let uri = code.to_uri()?;
    let qr = QrCode::new(uri.as_bytes())?
        .render::<Dense1x2>()
        // Inverted so it scans on the usual dark-background terminal
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(true)
        .build();
    println!(
        "\nScan with TOVI to pair (valid for {} seconds):\n",
        CODE_LIFETIME.as_secs()
    );
    println!("{qr}");
    println!("Or use the link:\n{uri}\n");
    Ok(())
}

async fn ask_user(request: PairingRequest, auto_approve: bool) -> bool {
    let prompt = format!(
        "\n{} ({}) wants to pair. Device ID {}.",
        request.device_name,
        request.platform,
        short(&request.device_id)
    );
    if auto_approve {
        println!("{prompt} Auto-approved.");
        return true;
    }
    tokio::task::spawn_blocking(move || {
        print!("{prompt}\nAllow? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).is_ok()
            && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    })
    .await
    .unwrap_or(false)
}

// ---------------------------------------------------------- pair / send

/// An open connection to a trusted device, and how to reach it again
struct Link {
    conn: PeerConnection,
    device: TrustedDevice,
    addresses: Vec<SocketAddr>,
}

/// This side only connects out, so it accepts no incoming connections
fn outgoing_endpoint(me: &Me) -> Result<QuicEndpoint> {
    QuicEndpoint::bind(
        me.identity.clone(),
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        Arc::new(|_: &DeviceId| false),
    )
}

/// Pair using a QR link and save the device and its addresses
async fn pair(me: &Me, uri: &str) -> Result<(QuicEndpoint, Link)> {
    let code = PairingCode::from_uri(uri.trim())?;
    let endpoint = outgoing_endpoint(me)?;
    println!("Connecting to {} address(es)...", code.endpoints.len());
    let (conn, device) = pairing::initiate(
        &endpoint,
        &code,
        &Hello::new(me.name.clone()),
        &me.identity.device_id(),
        me.store.as_ref(),
    )
    .await?;
    // The address that answered goes first next time
    let mut addresses = vec![conn.remote_address()];
    addresses.extend(
        code.endpoints
            .iter()
            .filter(|a| **a != conn.remote_address()),
    );
    me.store.set_device_addresses(&device.id, &addresses)?;
    println!(
        "Paired with {} at {} (saved; next time use its name)",
        describe(&device),
        conn.remote_address()
    );
    Ok((
        endpoint,
        Link {
            conn,
            device,
            addresses,
        },
    ))
}

/// Connect to an already-paired device at its saved addresses
async fn connect_paired(me: &Me, query: &str) -> Result<(QuicEndpoint, Link)> {
    let device = me
        .store
        .find_device(query)?
        .with_context(|| format!("no paired device matches {query:?}; see `tovi-cli devices`"))?;
    let addresses = me.store.device_addresses(&device.id)?;
    if addresses.is_empty() {
        bail!(
            "no known address for {}; pair again with its QR link",
            device.name
        );
    }
    let endpoint = outgoing_endpoint(me)?;
    println!("Connecting to {}...", describe(&device));
    let conn = endpoint
        .connect_any(&addresses, &device.id.public_key()?)
        .await
        .with_context(|| {
            format!(
                "could not reach {} at its last known address; is `tovi-cli listen` running there?",
                device.name
            )
        })?;
    me.store.touch_device(&device.id)?;
    Ok((
        endpoint,
        Link {
            conn,
            device,
            addresses,
        },
    ))
}

/// Send `file`, reconnecting and resuming if the connection drops. An
/// unfinished earlier send of the same file to the same device is resumed.
async fn send(me: &Me, file: &Path, to: &str) -> Result<()> {
    let (endpoint, link) = if to.trim().starts_with("tovi://") {
        pair(me, to).await?
    } else {
        connect_paired(me, to).await?
    };
    let device = &link.device;

    let options = SendOptions::default();
    let fresh = OutgoingTransfer::new(file, options)?;
    let transfer = match me.store.unfinished_send(
        &device.id,
        fresh.path(),
        fresh.file_size(),
        fresh.modified(),
    )? {
        Some(id) => {
            println!("Resuming an earlier, unfinished send of this file");
            OutgoingTransfer::with_id(file, options, id)?
        }
        None => fresh,
    };
    me.store
        .record_transfer_started(&transfer.history_record(&device.id))?;

    println!("Sending {} to {}...", file.display(), device.name);
    let size = transfer.file_size();
    let progress = ProgressPrinter::new("Sending");
    let result = transfer::send_with_resume(
        &endpoint,
        &link.addresses,
        &device.id.public_key()?,
        link.conn,
        &transfer,
        |p| progress.update(p),
        |e| println!("Connection lost ({e:#}); reconnecting to resume..."),
    )
    .await;

    let id = transfer.transfer_id();
    let outcome = match result {
        Ok((conn, hash)) => {
            me.store.record_transfer_completed(&id, None, &hash)?;
            conn.close();
            println!(
                "Sent and verified by {} ({}, {}). BLAKE3 {}",
                device.name,
                megabytes(size),
                progress.rate(size),
                &hash.to_hex()[..16]
            );
            Ok(())
        }
        Err(e) => {
            // A lost connection can be resumed later; anything else (refusal,
            // changed file, integrity failure) ends this transfer
            if e.downcast_ref::<ConnectionLost>().is_some() {
                println!("Run the same command again later to resume.");
            } else {
                me.store.record_transfer_failed(&id, &format!("{e:#}"))?;
            }
            Err(e)
        }
    };
    endpoint.shutdown().await;
    outcome
}

// ----------------------------------------------- devices / forget / history

fn show_devices(store: &Store) -> Result<()> {
    let devices = store.list()?;
    if devices.is_empty() {
        println!("No paired devices. Pair with `tovi-cli pair <link>` or `listen`.");
    }
    for device in devices {
        let addresses = store.device_addresses(&device.id)?;
        let addresses = match addresses.first() {
            Some(addr) => format!("last at {addr}"),
            None => "no known address".into(),
        };
        println!("{}  {addresses}", describe(&device));
    }
    Ok(())
}

fn forget(store: &Store, query: &str) -> Result<()> {
    let device = store
        .find_device(query)?
        .with_context(|| format!("no paired device matches {query:?}"))?;
    store.remove(&device.id)?;
    println!(
        "Forgot {}. It must pair again before it can send files.",
        describe(&device)
    );
    Ok(())
}

fn show_history(store: &Store, limit: usize) -> Result<()> {
    let names: std::collections::HashMap<DeviceId, String> =
        store.list()?.into_iter().map(|d| (d.id, d.name)).collect();
    let history = store.history(limit)?;
    if history.is_empty() {
        println!("No transfers yet.");
    }
    for t in history {
        let peer = names
            .get(&t.device_id)
            .cloned()
            .unwrap_or_else(|| short(&t.device_id));
        let arrow = match t.direction {
            Direction::Sent => format!("to {peer}"),
            Direction::Received => format!("from {peer}"),
        };
        let status = match t.status {
            TransferStatus::Completed => "done".to_string(),
            TransferStatus::InProgress => "unfinished".to_string(),
            TransferStatus::Failed => format!("failed: {}", t.error.unwrap_or_default()),
        };
        println!(
            "{}  {}  {}  {arrow}  [{status}]",
            ago(t.started_at),
            t.file_name,
            megabytes(t.file_size)
        );
    }
    Ok(())
}

fn ago(unix_secs: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match now.saturating_sub(unix_secs) {
        s if s < 60 => format!("{s:>3}s ago"),
        s if s < 3600 => format!("{:>3}m ago", s / 60),
        s if s < 86_400 => format!("{:>3}h ago", s / 3600),
        s => format!("{:>3}d ago", s / 86_400),
    }
}

// ----------------------------------------------------------- formatting

/// Prints progress every 10%, and the average speed at the end
struct ProgressPrinter {
    label: &'static str,
    started: Instant,
    last_tenth: AtomicU64,
}

impl ProgressPrinter {
    fn new(label: &'static str) -> Self {
        Self {
            label,
            started: Instant::now(),
            last_tenth: AtomicU64::new(0),
        }
    }

    fn update(&self, p: Progress) {
        if p.total == 0 {
            return;
        }
        let tenth = p.done * 10 / p.total;
        if tenth > self.last_tenth.swap(tenth, Ordering::Relaxed) {
            println!("  {} {}%", self.label, tenth * 10);
        }
    }

    fn rate(&self, bytes: u64) -> String {
        let secs = self.started.elapsed().as_secs_f64().max(0.001);
        format!("{:.1} MB/s", bytes as f64 / 1_000_000.0 / secs)
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

fn describe(device: &TrustedDevice) -> String {
    format!(
        "{} ({}, {})",
        device.name,
        device.platform,
        short(&device.id)
    )
}

/// First 8 hex characters of a device ID, for display
fn short(id: &DeviceId) -> String {
    id.to_string()[..8].to_string()
}
