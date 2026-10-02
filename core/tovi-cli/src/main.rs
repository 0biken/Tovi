//! Headless TOVI spike: pair two machines and send files from the terminal.
//!
//! Machine A (desktop role):  tovi-cli listen
//! Machine B (phone role):    tovi-cli send <file> "tovi://pair/..."

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tovi_core::identity::{DeviceId, DeviceIdentity, FileKeyStore};
use tovi_core::pairing::{self, PairingCode, PairingManager, PairingRequest, CODE_LIFETIME};
use tovi_core::protocol::{Hello, MAX_DEVICE_NAME_LEN};
use tovi_core::transfer::{self, Inbox, OutgoingTransfer, Progress, SendOptions};
use tovi_core::transport::{candidate_addresses, PeerConnection, QuicEndpoint};
use tovi_core::trust::{MemoryTrustStore, TrustStore, TrustedDevice};

/// Default UDP port for `listen` (Tech doc §13)
const DEFAULT_PORT: u16 = 48210;

#[derive(Parser)]
#[command(
    name = "tovi-cli",
    about = "TOVI headless spike: pair devices and send files from the terminal"
)]
struct Cli {
    /// Where this device's identity key is kept [default: OS data dir/TOVI]
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
    /// Pair with a device using its QR link, then send it a file (phone role)
    Send {
        /// File to send
        file: PathBuf,
        /// The `tovi://pair/...` link
        uri: String,
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
    let identity = Arc::new(DeviceIdentity::load_or_generate(&FileKeyStore::new(
        data_dir.join("identity.key"),
    ))?);
    let name = cli.name.unwrap_or_else(default_device_name);

    match cli.command {
        Command::Id => show_id(&identity, &name, &data_dir),
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
            listen(identity, name, port, receive_dir, auto_approve).await
        }
        Command::Pair { uri } => {
            let (endpoint, conn, _, _) = pair(identity, name, &uri).await?;
            conn.close();
            endpoint.shutdown().await;
            Ok(())
        }
        Command::Send { file, uri } => {
            let (endpoint, conn, device, code) = pair(identity, name, &uri).await?;
            let result = send(&endpoint, conn, &code, &file, &device).await;
            endpoint.shutdown().await;
            result
        }
    }
}

fn default_device_name() -> String {
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "TOVI device".into());
    name.chars().take(MAX_DEVICE_NAME_LEN).collect()
}

fn show_id(identity: &DeviceIdentity, name: &str, data_dir: &Path) -> Result<()> {
    println!("Name:       {name}");
    println!("Device ID:  {}", identity.device_id());
    println!(
        "Identity:   {} (development key file, unencrypted)",
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

struct Listener {
    manager: Arc<PairingManager>,
    hello: Hello,
    /// Shared by all connections, so a reconnecting sender can resume
    inbox: Inbox,
    auto_approve: bool,
}

async fn listen(
    identity: Arc<DeviceIdentity>,
    name: String,
    port: u16,
    receive_dir: PathBuf,
    auto_approve: bool,
) -> Result<()> {
    let trust: Arc<dyn TrustStore> = Arc::new(MemoryTrustStore::new());
    let manager = Arc::new(PairingManager::new(identity.public_key(), trust));
    let endpoint = QuicEndpoint::bind(
        identity.clone(),
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
    println!("{name} is listening on port {port}. Press Ctrl+C to stop.");
    println!("Received files go to {}", receive_dir.display());

    let listener = Arc::new(Listener {
        manager,
        hello: Hello::new(name),
        inbox: Inbox::new(receive_dir),
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

/// Untrusted devices may only pair; once trusted, they may send files
async fn handle_connection(listener: Arc<Listener>, conn: PeerConnection) {
    let manager = &listener.manager;
    let name = match manager
        .trust()
        .list()
        .into_iter()
        .find(|d| d.id == *conn.peer_id())
    {
        Some(device) => device.name,
        None => {
            let ask = |req| ask_user(req, listener.auto_approve);
            match pairing::respond(&conn, manager, &listener.hello, ask).await {
                Ok(device) => {
                    println!("Paired with {}", describe(&device));
                    device.name
                }
                Err(e) => {
                    println!("Pairing failed: {e:#}");
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
                        "{name} is sending {} ({}). Accepted: trusted device.",
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

/// Pair using a QR link; returns the open connection to the paired device
async fn pair(
    identity: Arc<DeviceIdentity>,
    name: String,
    uri: &str,
) -> Result<(QuicEndpoint, PeerConnection, TrustedDevice, PairingCode)> {
    let code = PairingCode::from_uri(uri.trim())?;
    // This side only connects out, so it accepts no incoming connections
    let endpoint = QuicEndpoint::bind(
        identity.clone(),
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        Arc::new(|_: &DeviceId| false),
    )?;
    let trust = MemoryTrustStore::new();

    println!("Connecting to {} address(es)...", code.endpoints.len());
    let (conn, device) = pairing::initiate(
        &endpoint,
        &code,
        &Hello::new(name),
        &identity.device_id(),
        &trust,
    )
    .await?;
    println!(
        "Paired with {} at {}",
        describe(&device),
        conn.remote_address()
    );
    Ok((endpoint, conn, device, code))
}

/// Send `file`, reconnecting (using the QR code's addresses and key) and
/// resuming if the connection drops
async fn send(
    endpoint: &QuicEndpoint,
    conn: PeerConnection,
    code: &PairingCode,
    file: &Path,
    device: &TrustedDevice,
) -> Result<()> {
    println!("Sending {} to {}...", file.display(), device.name);
    let transfer = OutgoingTransfer::new(file, SendOptions::default())?;
    let size = transfer.file_size();
    let progress = ProgressPrinter::new("Sending");
    let (conn, hash) = transfer::send_with_resume(
        endpoint,
        &code.endpoints,
        &code.desktop_key,
        conn,
        &transfer,
        |p| progress.update(p),
        |e| println!("Connection lost ({e:#}); reconnecting to resume..."),
    )
    .await?;
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
