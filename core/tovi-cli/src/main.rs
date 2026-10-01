//! Headless TOVI spike: pair two machines from the terminal.
//!
//! Machine A (desktop role):  tovi-cli listen
//! Machine B (phone role):    tovi-cli pair "tovi://pair/..."

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tovi_core::identity::{DeviceId, DeviceIdentity, FileKeyStore};
use tovi_core::pairing::{self, PairingCode, PairingManager, PairingRequest, CODE_LIFETIME};
use tovi_core::protocol::{Hello, MAX_DEVICE_NAME_LEN};
use tovi_core::transport::{candidate_addresses, QuicEndpoint};
use tovi_core::trust::{MemoryTrustStore, TrustStore};

/// Default UDP port for `listen` (Tech doc §13)
const DEFAULT_PORT: u16 = 48210;

#[derive(Parser)]
#[command(
    name = "tovi-cli",
    about = "TOVI headless spike: pair devices from the terminal"
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
    /// Wait for devices, showing a QR code to pair a new one (desktop role)
    Listen {
        /// UDP port to listen on
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        /// Accept pairing requests without asking. For automated testing only.
        #[arg(long)]
        auto_approve: bool,
    },
    /// Pair with a device using the link from its QR code (phone role)
    Pair {
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
        Command::Listen { port, auto_approve } => listen(identity, name, port, auto_approve).await,
        Command::Pair { uri } => pair(identity, name, &uri).await,
    }
}

fn default_device_name() -> String {
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "TOVI device".into());
    name.chars().take(MAX_DEVICE_NAME_LEN).collect()
}

fn show_id(identity: &DeviceIdentity, name: &str, data_dir: &std::path::Path) -> Result<()> {
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

async fn listen(
    identity: Arc<DeviceIdentity>,
    name: String,
    port: u16,
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

    let hello = Hello::new(name);
    loop {
        let incoming = tokio::select! {
            incoming = endpoint.accept() => incoming,
            _ = tokio::signal::ctrl_c() => break,
        };
        let Some(result) = incoming else { break };
        let conn = match result {
            Ok(conn) => conn,
            Err(e) => {
                println!("Refused a connection: {e:#}");
                continue;
            }
        };

        if manager.trust().is_trusted(conn.peer_id()) {
            println!(
                "Trusted device {} connected (file transfer not built yet)",
                short(conn.peer_id())
            );
            continue;
        }
        match pairing::respond(&conn, &manager, &hello, |req| ask_user(req, auto_approve)).await {
            Ok(device) => println!(
                "Paired with {} ({}, {})",
                device.name,
                device.platform,
                short(&device.id)
            ),
            Err(e) => println!("Pairing failed: {e:#}"),
        }
    }

    println!("Shutting down...");
    endpoint.shutdown().await;
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

async fn pair(identity: Arc<DeviceIdentity>, name: String, uri: &str) -> Result<()> {
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
        "Paired with {} ({}, {}) at {}",
        device.name,
        device.platform,
        short(&device.id),
        conn.remote_address()
    );

    conn.close();
    endpoint.shutdown().await;
    Ok(())
}

/// First 8 hex characters of a device ID, for display
fn short(id: &DeviceId) -> String {
    id.to_string()[..8].to_string()
}
