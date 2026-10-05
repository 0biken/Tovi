//! Headless TOVI spike: pair two machines and send files from the terminal.
//!
//! Machine A (desktop role):  tovi-cli listen
//! Machine B (phone role):    tovi-cli send <file> "tovi://pair/..."   (first time)
//!                            tovi-cli send <file> <device name>       (afterwards)
//!
//! `listen`, `pair` and `send` drive a [`Node`], the same engine the desktop
//! app uses, and print its events.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;
use std::collections::HashMap;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast::{self, error::RecvError};
use tovi_core::identity::{DeviceId, DeviceIdentity, FileKeyStore};
use tovi_core::node::{Event, Node, NodeConfig, TransferOutcome, DEFAULT_PORT};
use tovi_core::pairing::{PairingCode, CODE_LIFETIME};
use tovi_core::protocol::MAX_DEVICE_NAME_LEN;
use tovi_core::storage::{Direction, Store, TransferStatus};
use tovi_core::transfer::{ConnectionLost, TransferId};
use tovi_core::transport::candidate_addresses;
use tovi_core::trust::{TrustStore, TrustedDevice};

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
    let name = cli.name.unwrap_or_else(default_device_name);
    let store = || Store::open(&data_dir.join("tovi.db"));

    match cli.command {
        Command::Id => show_id(&data_dir, &name),
        Command::Listen {
            port,
            receive_dir,
            auto_approve,
        } => {
            let mut config = node_config(&data_dir, &name)?;
            config.listen = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
            config.receive_dir_override = receive_dir;
            config.auto_approve_pairing = auto_approve;
            listen(config).await
        }
        Command::Pair { uri } => {
            let (node, _events) = Node::start(outgoing_config(&data_dir, &name)?).await?;
            let result = pair(&node, &uri).await;
            node.shutdown().await;
            result.map(|_| ())
        }
        Command::Send { file, to } => send(outgoing_config(&data_dir, &name)?, &file, &to).await,
        Command::Devices => show_devices(&store()?),
        Command::Forget { device } => forget(&store()?, &device),
        Command::History { limit } => show_history(&store()?, limit),
    }
}

fn default_device_name() -> String {
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "TOVI device".into());
    name.chars().take(MAX_DEVICE_NAME_LEN).collect()
}

fn node_config(data_dir: &Path, name: &str) -> Result<NodeConfig> {
    let downloads = dirs::download_dir()
        .context("no Downloads folder; pass --receive-dir")?
        .join("TOVI");
    Ok(NodeConfig::new(data_dir, name, downloads))
}

/// For `pair` and `send`: any free port, so a `listen` on this machine keeps 48210
fn outgoing_config(data_dir: &Path, name: &str) -> Result<NodeConfig> {
    let mut config = node_config(data_dir, name)?;
    config.listen = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0));
    Ok(config)
}

fn show_id(data_dir: &Path, name: &str) -> Result<()> {
    let identity =
        DeviceIdentity::load_or_generate(&FileKeyStore::new(data_dir.join("identity.key")))?;
    println!("Name:       {name}");
    println!("Device ID:  {}", identity.device_id());
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

async fn listen(config: NodeConfig) -> Result<()> {
    let requested_port = config.listen.port();
    let auto_approve = config.auto_approve_pairing;
    let (node, mut events) = Node::start(config).await?;
    let port = node.local_addr()?.port();

    print_pairing_code(&node.new_pairing_code()?)?;
    if auto_approve {
        println!("WARNING: --auto-approve is on; any device with this code will be trusted.\n");
    }
    if port != requested_port {
        println!("Port {requested_port} is in use, so this uses port {port}.");
    }
    let paired = node.devices()?.len();
    println!(
        "{} is listening on port {port}. {paired} paired device(s) can send without a code.",
        node.device_name()
    );
    println!(
        "Received files go to {}. Press Ctrl+C to stop.",
        node.receive_dir().display()
    );

    let mut printer = EventPrinter::default();
    loop {
        let event = tokio::select! {
            event = events.recv() => event,
            _ = tokio::signal::ctrl_c() => break,
        };
        match event {
            Ok(event) => printer.print(&node, event),
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => break,
        }
    }

    println!("Shutting down...");
    node.shutdown().await;
    Ok(())
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

/// Ask a yes/no question on the terminal without blocking event handling,
/// and pass the answer to the node
fn ask_in_background(node: &Node, request_id: u64, question: String) {
    let node = node.clone();
    tokio::spawn(async move {
        let allow = tokio::task::spawn_blocking(move || {
            print!("{question} [y/N] ");
            let _ = std::io::stdout().flush();
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer).is_ok()
                && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
        })
        .await
        .unwrap_or(false);
        node.respond(request_id, allow);
    });
}

/// Turns node events into terminal output
#[derive(Default)]
struct EventPrinter {
    /// Last 10% step printed, per transfer
    progress: HashMap<TransferId, u64>,
}

impl EventPrinter {
    fn print(&mut self, node: &Node, event: Event) {
        match event {
            Event::PairingRequested {
                request_id,
                request,
            } => ask_in_background(
                node,
                request_id,
                format!(
                    "\n{} ({}) wants to pair. Device ID {}.\nAllow?",
                    request.device_name,
                    request.platform,
                    short(&request.device_id)
                ),
            ),
            Event::Paired { device } => println!("Paired with {} (saved)", describe(&device)),
            Event::PairingFailed { device_id, error } => {
                println!("Refused {}: not paired ({error})", short(&device_id))
            }
            Event::IncomingOffer {
                request_id,
                from,
                file_name,
                file_size,
                ..
            } => ask_in_background(
                node,
                request_id,
                format!(
                    "\n{} wants to send {file_name} ({}).\nAccept?",
                    from.name,
                    megabytes(file_size)
                ),
            ),
            Event::RequestExpired { .. } => println!("No answer in time; declined."),
            Event::TransferStarted {
                direction,
                device,
                file_name,
                file_size,
                resuming,
                ..
            } => match direction {
                Direction::Received => println!(
                    "{} is sending {file_name} ({}).",
                    device.name,
                    megabytes(file_size)
                ),
                Direction::Sent => {
                    if resuming {
                        println!("Resuming an earlier, unfinished send of this file");
                    }
                    println!("Sending {file_name} to {}...", device.name);
                }
            },
            Event::TransferProgress {
                transfer_id,
                direction,
                done,
                total,
                ..
            } => {
                if total == 0 {
                    return;
                }
                let tenth = done * 10 / total;
                let last = self.progress.entry(transfer_id).or_default();
                if tenth > *last {
                    *last = tenth;
                    let label = match direction {
                        Direction::Sent => "Sending",
                        Direction::Received => "Receiving",
                    };
                    println!("  {label} {}%", tenth * 10);
                }
            }
            Event::TransferReconnecting { error, .. } => {
                println!("Connection lost ({error}); reconnecting to resume...")
            }
            Event::TransferFinished {
                transfer_id,
                direction,
                device,
                outcome,
                ..
            } => {
                self.progress.remove(&transfer_id);
                print_finished(direction, &device, outcome);
            }
            Event::DevicesChanged => {}
        }
    }
}

fn print_finished(direction: Direction, device: &TrustedDevice, outcome: TransferOutcome) {
    match (direction, outcome) {
        (
            Direction::Received,
            TransferOutcome::Completed {
                path,
                size,
                resumed_bytes,
                elapsed,
                ..
            },
        ) => {
            let resumed = match resumed_bytes {
                0 => String::new(),
                bytes if bytes == size => ", already received earlier".into(),
                bytes => format!(", resumed with {} already here", megabytes(bytes)),
            };
            let path = path.map(|p| p.display().to_string()).unwrap_or_default();
            println!(
                "Saved {path} ({}, {}{resumed})",
                megabytes(size),
                rate(size - resumed_bytes, elapsed)
            );
        }
        (
            Direction::Sent,
            TransferOutcome::Completed {
                size,
                file_hash,
                elapsed,
                ..
            },
        ) => println!(
            "Sent and verified by {} ({}, {}). BLAKE3 {}",
            device.name,
            megabytes(size),
            rate(size, elapsed),
            &file_hash.to_hex()[..16]
        ),
        (Direction::Received, TransferOutcome::Interrupted { error }) => println!(
            "Connection to {} lost ({error}); kept the partial file so it can resume",
            device.name
        ),
        (Direction::Received, TransferOutcome::Failed { error }) => {
            println!("Receive failed: {error}")
        }
        // The sender prints its own failure from the returned error
        (Direction::Sent, _) => {}
    }
}

// ---------------------------------------------------------- pair / send

/// Pair using a QR link; the node saves the device and its addresses
async fn pair(node: &Node, uri: &str) -> Result<TrustedDevice> {
    let code = PairingCode::from_uri(uri.trim())?;
    println!("Connecting to {} address(es)...", code.endpoints.len());
    let device = node.pair(uri).await?;
    let at = node
        .devices()?
        .into_iter()
        .find(|d| d.device.id == device.id)
        .and_then(|d| d.addresses.first().copied())
        .map(|a| format!(" at {a}"))
        .unwrap_or_default();
    println!(
        "Paired with {}{at} (saved; next time use its name)",
        describe(&device)
    );
    Ok(device)
}

/// Send `file`, pairing first if `to` is a link. The node reconnects and
/// resumes after drops, and resumes an unfinished earlier send of the file.
async fn send(config: NodeConfig, file: &Path, to: &str) -> Result<()> {
    let (node, mut events) = Node::start(config).await?;
    let result = send_with(&node, &mut events, file, to).await;
    node.shutdown().await;
    result
}

async fn send_with(
    node: &Node,
    events: &mut broadcast::Receiver<Event>,
    file: &Path,
    to: &str,
) -> Result<()> {
    let device = if to.trim().starts_with("tovi://") {
        let device = pair(node, to).await?;
        // `pair` already reported the pairing; skip its events
        while events.try_recv().is_ok() {}
        device
    } else {
        let device = node
            .find_device(to)?
            .with_context(|| format!("no paired device matches {to:?}; see `tovi-cli devices`"))?;
        println!("Connecting to {}...", describe(&device));
        device
    };

    // Print the node's events while the send runs, then any left over
    let mut printer = EventPrinter::default();
    let sending = node.send_file(&device.id, file);
    tokio::pin!(sending);
    let result = loop {
        tokio::select! {
            biased;
            Ok(event) = events.recv() => printer.print(node, event),
            result = &mut sending => break result,
        }
    };
    while let Ok(event) = events.try_recv() {
        printer.print(node, event);
    }

    if let Err(e) = &result {
        if e.downcast_ref::<ConnectionLost>().is_some() {
            println!("Run the same command again later to resume.");
        }
    }
    result.map(|_| ())
}

fn rate(bytes: u64, elapsed: Duration) -> String {
    let secs = elapsed.as_secs_f64().max(0.001);
    format!("{:.1} MB/s", bytes as f64 / 1_000_000.0 / secs)
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
