//! QUIC transport. Phase 1: LAN via `quinn`.
//!
//! Every TOVI link, whatever carries it (LAN today; Wi-Fi Direct, Wi-Fi Aware
//! or a UDP relay later), is an IP path. So there is one secure transport,
//! [`QuicEndpoint`], and future transports supply *addresses* to it instead of
//! reimplementing encryption and authentication.
//!
//! Trust (decisions.md D1):
//! - Outgoing: the server's key is pinned by the TLS config; a mismatch fails
//!   the handshake.
//! - Incoming: TLS accepts any valid Ed25519 client certificate, then
//!   [`QuicEndpoint::accept`] asks the [`PeerAuthorizer`] whether that device
//!   is allowed (trusted, or in an active pairing session). Rejected
//!   connections are closed with [`CLOSE_UNAUTHORIZED`] and never returned.

use crate::identity::tls::{public_key_from_cert, SERVER_NAME};
use crate::identity::{DeviceId, DeviceIdentity};
use anyhow::{anyhow, Context, Result};
use ed25519_dalek::VerifyingKey;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ConnectionError, IdleTimeout, RecvStream, SendStream, TransportConfig, VarInt};
use rustls::pki_types::CertificateDer;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

/// ALPN protocol ID; also the TVP protocol version on the wire
pub const ALPN: &[u8] = b"tvp/1";

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
pub const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(10);

/// Application close codes
pub const CLOSE_NORMAL: VarInt = VarInt::from_u32(0);
pub const CLOSE_UNAUTHORIZED: VarInt = VarInt::from_u32(1);

/// Decides whether an authenticated device may connect to us.
///
/// Implemented by the trusted-device store and pairing sessions. Any
/// `Fn(&DeviceId) -> bool` also works, which is handy in tests and the CLI.
pub trait PeerAuthorizer: Send + Sync {
    fn authorize(&self, peer: &DeviceId) -> bool;
}

impl<F: Fn(&DeviceId) -> bool + Send + Sync> PeerAuthorizer for F {
    fn authorize(&self, peer: &DeviceId) -> bool {
        self(peer)
    }
}

/// An incoming connection from a device the [`PeerAuthorizer`] rejected
#[derive(Debug)]
pub struct UnauthorizedPeer(pub DeviceId);

impl fmt::Display for UnauthorizedPeer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "device {} is not authorized", self.0)
    }
}

impl std::error::Error for UnauthorizedPeer {}

/// A QUIC endpoint that both accepts and makes TOVI connections
pub struct QuicEndpoint {
    endpoint: quinn::Endpoint,
    identity: Arc<DeviceIdentity>,
    transport: Arc<TransportConfig>,
    authorizer: Arc<dyn PeerAuthorizer>,
}

impl QuicEndpoint {
    /// Bind a UDP socket at `addr` (use port 0 for any free port)
    pub fn bind(
        identity: Arc<DeviceIdentity>,
        addr: SocketAddr,
        authorizer: Arc<dyn PeerAuthorizer>,
    ) -> Result<Self> {
        let transport = Arc::new(transport_config()?);

        let mut tls = identity.server_config()?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        let mut server =
            quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
        server.transport_config(transport.clone());

        let endpoint = quinn::Endpoint::server(server, addr)
            .with_context(|| format!("binding QUIC endpoint on {addr}"))?;
        Ok(Self {
            endpoint,
            identity,
            transport,
            authorizer,
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    /// Connect to the device at `addr` whose identity key must be `server_key`
    pub async fn connect(
        &self,
        addr: SocketAddr,
        server_key: &VerifyingKey,
    ) -> Result<PeerConnection> {
        self.connect_any(&[addr], server_key).await
    }

    /// Try every address at once and keep the first connection that completes
    /// the handshake with `server_key`; the other attempts are abandoned.
    /// Used with the QR code's candidate endpoints (decisions.md D3).
    pub async fn connect_any(
        &self,
        addrs: &[SocketAddr],
        server_key: &VerifyingKey,
    ) -> Result<PeerConnection> {
        let mut tls = self.identity.client_config(*server_key)?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        let mut client = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(tls)?));
        client.transport_config(self.transport.clone());

        let mut last_error = anyhow!("no addresses to connect to");
        let mut attempts = tokio::task::JoinSet::new();
        for &addr in addrs {
            // An unusable address (e.g. IPv6 on an IPv4 socket) skips just that candidate
            let connecting = match self
                .endpoint
                .connect_with(client.clone(), addr, SERVER_NAME)
            {
                Ok(connecting) => connecting,
                Err(e) => {
                    last_error = anyhow!(e).context(format!("connecting to {addr}"));
                    continue;
                }
            };
            attempts.spawn(async move {
                tokio::time::timeout(CONNECT_TIMEOUT, connecting)
                    .await
                    .map_err(|_| anyhow!("timed out connecting to {addr}"))?
                    .with_context(|| format!("connecting to {addr}"))
            });
        }

        while let Some(result) = attempts.join_next().await {
            match result? {
                Ok(conn) => return PeerConnection::new(conn), // dropping `attempts` aborts the rest
                Err(e) => last_error = e,
            }
        }
        Err(last_error)
    }

    /// Wait for the next incoming connection.
    ///
    /// Returns `None` once the endpoint is closed. An `Err` means that one
    /// attempt failed (handshake error, timeout, or [`UnauthorizedPeer`]);
    /// keep calling `accept` for the next one.
    pub async fn accept(&self) -> Option<Result<PeerConnection>> {
        let incoming = self.endpoint.accept().await?;
        Some(self.finish_accept(incoming).await)
    }

    async fn finish_accept(&self, incoming: quinn::Incoming) -> Result<PeerConnection> {
        let remote = incoming.remote_address();
        let conn = tokio::time::timeout(CONNECT_TIMEOUT, incoming)
            .await
            .map_err(|_| anyhow!("handshake with {remote} timed out"))??;
        let peer = PeerConnection::new(conn)?;

        if !self.authorizer.authorize(&peer.peer_id) {
            peer.inner.close(CLOSE_UNAUTHORIZED, b"unauthorized");
            return Err(UnauthorizedPeer(peer.peer_id).into());
        }
        Ok(peer)
    }

    /// Close all connections and wait for peers to be notified
    pub async fn shutdown(&self) {
        self.endpoint.close(CLOSE_NORMAL, b"shutdown");
        self.endpoint.wait_idle().await;
    }
}

fn transport_config() -> Result<TransportConfig> {
    let mut config = TransportConfig::default();
    config
        .max_idle_timeout(Some(IdleTimeout::try_from(IDLE_TIMEOUT)?))
        .keep_alive_interval(Some(KEEP_ALIVE_INTERVAL));
    Ok(config)
}

/// An authenticated connection to another TOVI device. Cheap to clone; clones
/// share the same connection.
#[derive(Clone)]
pub struct PeerConnection {
    inner: quinn::Connection,
    peer_id: DeviceId,
}

impl PeerConnection {
    fn new(inner: quinn::Connection) -> Result<Self> {
        let certs = inner
            .peer_identity()
            .and_then(|id| id.downcast::<Vec<CertificateDer<'static>>>().ok())
            .ok_or_else(|| anyhow!("peer presented no certificate"))?;
        let cert = certs
            .first()
            .ok_or_else(|| anyhow!("peer presented no certificate"))?;
        let peer_id = DeviceId::from(&public_key_from_cert(cert)?);
        Ok(Self { inner, peer_id })
    }

    /// The authenticated identity of the other device
    pub fn peer_id(&self) -> &DeviceId {
        &self.peer_id
    }

    pub fn remote_address(&self) -> SocketAddr {
        self.inner.remote_address()
    }

    pub async fn open_bi(&self) -> Result<(SendStream, RecvStream)> {
        Ok(self.inner.open_bi().await?)
    }

    pub async fn accept_bi(&self) -> Result<(SendStream, RecvStream)> {
        Ok(self.inner.accept_bi().await?)
    }

    /// Like [`Self::accept_bi`], but `Ok(None)` once the connection has been
    /// closed normally by either side
    pub async fn next_bi(&self) -> Result<Option<(SendStream, RecvStream)>> {
        match self.inner.accept_bi().await {
            Ok(streams) => Ok(Some(streams)),
            Err(ConnectionError::ApplicationClosed(_) | ConnectionError::LocallyClosed) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub async fn open_uni(&self) -> Result<SendStream> {
        Ok(self.inner.open_uni().await?)
    }

    pub async fn accept_uni(&self) -> Result<RecvStream> {
        Ok(self.inner.accept_uni().await?)
    }

    /// 32 bytes of keying material unique to this TLS session; both sides
    /// derive the same value. Used to bind pairing proofs to the connection
    /// (decisions.md D2).
    pub fn export_keying_material(&self, label: &[u8], context: &[u8]) -> Result<[u8; 32]> {
        let mut out = [0u8; 32];
        self.inner
            .export_keying_material(&mut out, label, context)
            .map_err(|_| anyhow!("TLS keying material export failed"))?;
        Ok(out)
    }

    pub fn close(&self) {
        self.inner.close(CLOSE_NORMAL, b"done");
    }

    /// Wait until the connection is closed, by either side, and return why
    pub async fn closed(&self) -> ConnectionError {
        self.inner.closed().await
    }

    /// Whether the other side closed this connection because it does not
    /// authorize us ([`CLOSE_UNAUTHORIZED`])
    pub fn was_refused(&self) -> bool {
        matches!(
            self.inner.close_reason(),
            Some(ConnectionError::ApplicationClosed(close)) if close.error_code == CLOSE_UNAUTHORIZED
        )
    }
}

/// Addresses on this machine that a phone on the same network could reach us
/// at, best first. Used for the QR code's endpoint list (decisions.md D3).
///
/// IPv4 only for now, matching the IPv4 bind address.
pub fn candidate_addresses(port: u16) -> Result<Vec<SocketAddr>> {
    let interfaces = if_addrs::get_if_addrs()?;
    Ok(rank_candidates(
        interfaces.iter().map(|i| (i.name.as_str(), i.ip())),
        port,
    ))
}

/// Interface name fragments for virtual, container and VPN adapters. Their
/// addresses are unreachable from a phone on the LAN.
const VIRTUAL_NAME_PREFIXES: &[&str] = &[
    "docker", "br-", "veth", "vmnet", "vboxnet", "tun", "tap", "utun", "wg",
];
const VIRTUAL_NAME_FRAGMENTS: &[&str] = &[
    "vethernet",
    "wsl",
    "hyper-v",
    "virtualbox",
    "vmware",
    "tailscale",
    "zerotier",
    "wireguard",
];

fn is_virtual_interface(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    VIRTUAL_NAME_PREFIXES.iter().any(|p| name.starts_with(p))
        || VIRTUAL_NAME_FRAGMENTS.iter().any(|f| name.contains(f))
}

fn rank_candidates<'a>(
    interfaces: impl IntoIterator<Item = (&'a str, IpAddr)>,
    port: u16,
) -> Vec<SocketAddr> {
    let mut v4: Vec<Ipv4Addr> = interfaces
        .into_iter()
        .filter(|(name, _)| !is_virtual_interface(name))
        .filter_map(|(_, ip)| match ip {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(_) => None,
        })
        .filter(|ip| !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified())
        .collect();
    // Home-network (RFC 1918) addresses first; stable sort keeps OS order otherwise
    v4.sort_by_key(|ip| !ip.is_private());
    v4.dedup();
    v4.into_iter()
        .map(|ip| SocketAddr::new(IpAddr::V4(ip), port))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCALHOST: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

    fn endpoint(identity: &Arc<DeviceIdentity>, allow: bool) -> QuicEndpoint {
        QuicEndpoint::bind(
            identity.clone(),
            LOCALHOST,
            Arc::new(move |_: &DeviceId| allow),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn connects_authenticates_and_exchanges_data() {
        let desktop = Arc::new(DeviceIdentity::generate_new());
        let phone = Arc::new(DeviceIdentity::generate_new());
        let server = endpoint(&desktop, true);
        let client = endpoint(&phone, true);
        let server_addr = server.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let conn = server.accept().await.unwrap().unwrap();
            let (mut send, mut recv) = conn.accept_bi().await.unwrap();
            let msg = recv.read_to_end(1024).await.unwrap();
            send.write_all(&msg).await.unwrap();
            send.finish().unwrap();
            conn // keep the connection open until the client has read the echo
        });

        let conn = client
            .connect(server_addr, &desktop.public_key())
            .await
            .unwrap();
        assert_eq!(conn.peer_id(), &desktop.device_id());

        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        send.write_all(b"hello tovi").await.unwrap();
        send.finish().unwrap();
        assert_eq!(recv.read_to_end(1024).await.unwrap(), b"hello tovi");

        let server_conn = server_task.await.unwrap();
        assert_eq!(server_conn.peer_id(), &phone.device_id());
    }

    #[tokio::test]
    async fn connect_any_finds_the_reachable_address() {
        let desktop = Arc::new(DeviceIdentity::generate_new());
        let phone = Arc::new(DeviceIdentity::generate_new());
        let server = endpoint(&desktop, true);
        let client = endpoint(&phone, true);
        let live = server.local_addr().unwrap();
        // A port nobody listens on, and an address family this socket can't use
        let dead = std::net::UdpSocket::bind(LOCALHOST)
            .unwrap()
            .local_addr()
            .unwrap();
        let wrong_family: SocketAddr = "[::1]:9".parse().unwrap();

        let server_task = tokio::spawn(async move { server.accept().await.unwrap().is_ok() });
        let started = std::time::Instant::now();
        let conn = client
            .connect_any(&[dead, wrong_family, live], &desktop.public_key())
            .await
            .unwrap();

        assert_eq!(conn.remote_address(), live);
        // Didn't wait for the dead address to time out
        assert!(started.elapsed() < CONNECT_TIMEOUT / 2);
        assert!(server_task.await.unwrap());
    }

    #[tokio::test]
    async fn connect_any_reports_failure_when_nothing_answers() {
        let desktop = Arc::new(DeviceIdentity::generate_new());
        let client = endpoint(&Arc::new(DeviceIdentity::generate_new()), true);
        let wrong_family: SocketAddr = "[::1]:9".parse().unwrap();

        assert!(client
            .connect_any(&[], &desktop.public_key())
            .await
            .is_err());
        assert!(client
            .connect_any(&[wrong_family], &desktop.public_key())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn connect_fails_when_server_key_does_not_match() {
        let desktop = Arc::new(DeviceIdentity::generate_new());
        let impostor = Arc::new(DeviceIdentity::generate_new());
        let phone = Arc::new(DeviceIdentity::generate_new());
        let server = endpoint(&impostor, true);
        let client = endpoint(&phone, true);
        let server_addr = server.local_addr().unwrap();

        let server_task = tokio::spawn(async move { server.accept().await.unwrap().is_err() });

        assert!(client
            .connect(server_addr, &desktop.public_key())
            .await
            .is_err());
        assert!(
            server_task.await.unwrap(),
            "server must not accept the connection"
        );
    }

    #[tokio::test]
    async fn unauthorized_client_is_rejected() {
        let desktop = Arc::new(DeviceIdentity::generate_new());
        let stranger = Arc::new(DeviceIdentity::generate_new());
        let server = endpoint(&desktop, false);
        let client = endpoint(&stranger, true);
        let server_addr = server.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let err = server.accept().await.unwrap().err().unwrap();
            let rejected = err.downcast_ref::<UnauthorizedPeer>().unwrap().0;
            (rejected, server) // keep the endpoint alive so the close reaches the client
        });

        // TLS 1.3 lets the client finish its side of the handshake before the
        // server decides, so the close may arrive during or after connect
        let close = match client.connect(server_addr, &desktop.public_key()).await {
            Ok(conn) => conn.closed().await,
            Err(e) => e.downcast::<ConnectionError>().unwrap(),
        };
        match close {
            ConnectionError::ApplicationClosed(c) => assert_eq!(c.error_code, CLOSE_UNAUTHORIZED),
            other => panic!("expected an unauthorized close, got {other:?}"),
        }

        let (rejected, _server) = server_task.await.unwrap();
        assert_eq!(rejected, stranger.device_id());
    }

    #[tokio::test]
    async fn both_sides_export_the_same_keying_material() {
        let desktop = Arc::new(DeviceIdentity::generate_new());
        let phone = Arc::new(DeviceIdentity::generate_new());
        let server = endpoint(&desktop, true);
        let client = endpoint(&phone, true);
        let server_addr = server.local_addr().unwrap();

        let server_task = tokio::spawn(async move {
            let conn = server.accept().await.unwrap().unwrap();
            conn.export_keying_material(b"EXPORTER-TOVI-TEST", b"")
                .unwrap()
        });
        let conn = client
            .connect(server_addr, &desktop.public_key())
            .await
            .unwrap();
        let client_secret = conn
            .export_keying_material(b"EXPORTER-TOVI-TEST", b"")
            .unwrap();

        assert_eq!(client_secret, server_task.await.unwrap());
        assert_ne!(client_secret, [0u8; 32]);
    }

    #[test]
    fn candidates_skip_loopback_link_local_and_virtual_adapters() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        // Interface list from a real Windows dev machine, plus common virtual adapters
        let interfaces = [
            ("Bluetooth Network Connection", ip("169.254.253.27")),
            ("Wi-Fi 2", ip("10.175.169.49")),
            ("Local Area Connection* 2", ip("169.254.141.169")),
            ("Loopback Pseudo-Interface 1", ip("127.0.0.1")),
            ("vEthernet (WSL)", ip("172.28.160.1")),
            ("docker0", ip("172.17.0.1")),
            ("Tailscale", ip("100.101.102.103")),
            ("Wi-Fi 2", ip("fe80::1")),
        ];

        assert_eq!(
            rank_candidates(interfaces, 48210),
            vec!["10.175.169.49:48210".parse::<SocketAddr>().unwrap()]
        );
    }

    #[test]
    fn candidates_from_this_machine_are_usable() {
        // Runs the real OS interface query; the result depends on the machine
        for addr in candidate_addresses(48210).unwrap() {
            println!("candidate: {addr}");
            assert_eq!(addr.port(), 48210);
            let IpAddr::V4(ip) = addr.ip() else {
                panic!("expected IPv4, got {addr}")
            };
            assert!(
                !ip.is_loopback() && !ip.is_link_local(),
                "unusable candidate {addr}"
            );
        }
    }

    #[test]
    fn candidates_prefer_private_addresses() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let interfaces = [("en1", ip("203.0.113.5")), ("en0", ip("192.168.1.20"))];

        let ranked = rank_candidates(interfaces, 1);
        assert_eq!(ranked[0].ip(), ip("192.168.1.20"));
        assert_eq!(ranked[1].ip(), ip("203.0.113.5"));
    }
}
