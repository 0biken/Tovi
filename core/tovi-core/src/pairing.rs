//! QR pairing (decisions.md D2, D3).
//!
//! 1. The desktop opens a pairing session and shows a QR code holding its
//!    public key, a one-time secret, an absolute expiry and its addresses.
//! 2. The phone connects, pinning the desktop key from the QR.
//! 3. Both exchange `HELLO`; the phone sends `PAIR` with a proof: a BLAKE3
//!    keyed hash, keyed by the secret, over a value exported from this TLS
//!    session and both public keys. A proof can't be replayed on, or relayed
//!    through, another connection.
//! 4. The desktop checks the proof, then asks the user (Allow / Cancel).
//!    Only on Allow do both sides store each other as trusted.
//!
//! While a session is open, [`PairingManager`] lets *unknown* devices connect
//! so they can pair. The connection handler must allow such devices nothing
//! but this pairing exchange.

use crate::identity::DeviceId;
use crate::protocol::{self, Hello, Message, Pair, PairResult};
use crate::transport::{PeerAuthorizer, PeerConnection, QuicEndpoint};
use crate::trust::{TrustStore, TrustedDevice};
use anyhow::{anyhow, bail, ensure, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ed25519_dalek::VerifyingKey;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a QR code stays valid (decisions.md D3)
pub const CODE_LIFETIME: Duration = Duration::from_secs(60);
/// Failed proofs before an open session is closed (decisions.md D2)
pub const MAX_FAILED_ATTEMPTS: u32 = 3;
/// The phone only rejects a code early if it expired longer ago than this,
/// so a phone with a slightly wrong clock still tries. The desktop enforces
/// the real expiry.
pub const CLOCK_SKEW_ALLOWANCE: Duration = Duration::from_secs(120);

const URI_PREFIX: &str = "tovi://pair/";
const CODE_VERSION: u8 = 1;
const MAX_ENDPOINTS: usize = 8;
const PROOF_CONTEXT: &[u8] = b"TOVI-PAIR-v1";
const EXPORTER_LABEL: &[u8] = b"EXPORTER-TOVI-PAIR-v1";

/// Everything in a pairing QR code
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub desktop_key: VerifyingKey,
    pub secret: [u8; 32],
    /// Unix seconds
    pub expires_at: u64,
    pub endpoints: Vec<SocketAddr>,
}

/// CBOR layout of the QR payload, short keys to keep the QR small
#[derive(Serialize, Deserialize)]
struct WireCode {
    v: u8,
    #[serde(with = "serde_bytes")]
    k: [u8; 32],
    #[serde(with = "serde_bytes")]
    s: [u8; 32],
    x: u64,
    e: Vec<String>,
}

impl PairingCode {
    /// The `tovi://pair/...` URI to render as a QR code
    pub fn to_uri(&self) -> Result<String> {
        let wire = WireCode {
            v: CODE_VERSION,
            k: self.desktop_key.to_bytes(),
            s: self.secret,
            x: self.expires_at,
            e: self.endpoints.iter().map(SocketAddr::to_string).collect(),
        };
        let mut cbor = Vec::new();
        ciborium::into_writer(&wire, &mut cbor)?;
        Ok(format!("{URI_PREFIX}{}", URL_SAFE_NO_PAD.encode(cbor)))
    }

    /// Parse a scanned URI. Does not check expiry; see [`Self::check_not_expired`].
    pub fn from_uri(uri: &str) -> Result<Self> {
        let payload = uri
            .strip_prefix(URI_PREFIX)
            .ok_or_else(|| anyhow!("not a TOVI pairing code"))?;
        let cbor = URL_SAFE_NO_PAD
            .decode(payload)
            .context("corrupt pairing code")?;
        let wire: WireCode =
            ciborium::from_reader(cbor.as_slice()).context("corrupt pairing code")?;
        ensure!(
            wire.v == CODE_VERSION,
            "unsupported pairing code version {}",
            wire.v
        );
        ensure!(
            !wire.e.is_empty() && wire.e.len() <= MAX_ENDPOINTS,
            "pairing code has {} endpoints",
            wire.e.len()
        );

        Ok(Self {
            desktop_key: VerifyingKey::from_bytes(&wire.k).context("invalid desktop key")?,
            secret: wire.s,
            expires_at: wire.x,
            endpoints: wire
                .e
                .iter()
                .map(|e| e.parse().with_context(|| format!("invalid endpoint {e:?}")))
                .collect::<Result<_>>()?,
        })
    }

    /// Phone-side early check, so an old screenshot fails with a clear message
    pub fn check_not_expired(&self) -> Result<()> {
        let deadline = self.expires_at + CLOCK_SKEW_ALLOWANCE.as_secs();
        ensure!(unix_now() <= deadline, "this pairing code has expired");
        Ok(())
    }
}

/// The value both sides compute to prove knowledge of the QR secret, bound to
/// one TLS session and both identities (decisions.md D2)
pub fn pairing_proof(
    secret: &[u8; 32],
    exporter: &[u8; 32],
    phone: &DeviceId,
    desktop: &DeviceId,
) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new_keyed(secret);
    hasher.update(PROOF_CONTEXT);
    hasher.update(exporter);
    hasher.update(phone.as_bytes());
    hasher.update(desktop.as_bytes());
    hasher.finalize()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_secs()
}

struct Session {
    secret: [u8; 32],
    expires_at: u64,
    failed_attempts: u32,
}

/// Desktop side: open pairing sessions plus the trusted-device store
pub struct PairingManager {
    desktop_key: VerifyingKey,
    trust: Arc<dyn TrustStore>,
    sessions: Mutex<Vec<Session>>,
}

/// What the desktop user is asked to approve
#[derive(Debug, Clone)]
pub struct PairingRequest {
    pub device_id: DeviceId,
    pub device_name: String,
    pub platform: String,
}

impl PairingManager {
    pub fn new(desktop_key: VerifyingKey, trust: Arc<dyn TrustStore>) -> Self {
        Self {
            desktop_key,
            trust,
            sessions: Mutex::new(Vec::new()),
        }
    }

    pub fn trust(&self) -> &Arc<dyn TrustStore> {
        &self.trust
    }

    /// Open a pairing session; render the returned code's URI as a QR code
    pub fn start_session(&self, endpoints: Vec<SocketAddr>) -> PairingCode {
        self.start_session_with_lifetime(endpoints, CODE_LIFETIME)
    }

    fn start_session_with_lifetime(
        &self,
        endpoints: Vec<SocketAddr>,
        lifetime: Duration,
    ) -> PairingCode {
        let mut secret = [0u8; 32];
        OsRng.fill_bytes(&mut secret);
        let expires_at = unix_now() + lifetime.as_secs();
        self.sessions.lock().unwrap().push(Session {
            secret,
            expires_at,
            failed_attempts: 0,
        });
        PairingCode {
            desktop_key: self.desktop_key,
            secret,
            expires_at,
            endpoints,
        }
    }

    /// Whether any session is still open (unknown devices may connect to pair)
    pub fn is_pairing_open(&self) -> bool {
        let mut sessions = self.sessions.lock().unwrap();
        prune_expired(&mut sessions);
        !sessions.is_empty()
    }

    /// Check a `PAIR` proof against every open session. A match consumes that
    /// session (single use); a miss counts as a failed attempt on all of them.
    fn verify_and_consume(&self, proof: &[u8; 32], exporter: &[u8; 32], phone: &DeviceId) -> bool {
        let desktop = DeviceId::from(&self.desktop_key);
        let claimed = blake3::Hash::from_bytes(*proof);
        let mut sessions = self.sessions.lock().unwrap();
        prune_expired(&mut sessions);

        // blake3::Hash equality is constant-time
        let matched = sessions
            .iter()
            .position(|s| pairing_proof(&s.secret, exporter, phone, &desktop) == claimed);
        match matched {
            Some(index) => {
                sessions.remove(index);
                true
            }
            None => {
                for session in sessions.iter_mut() {
                    session.failed_attempts += 1;
                }
                sessions.retain(|s| s.failed_attempts < MAX_FAILED_ATTEMPTS);
                false
            }
        }
    }
}

fn prune_expired(sessions: &mut Vec<Session>) {
    let now = unix_now();
    sessions.retain(|s| now <= s.expires_at);
}

impl PeerAuthorizer for PairingManager {
    fn authorize(&self, peer: &DeviceId) -> bool {
        self.trust.is_trusted(peer) || self.is_pairing_open()
    }
}

/// Desktop: run the pairing exchange on an incoming connection from an
/// unknown device. `approve` shows the Allow / Cancel prompt.
///
/// Returns the newly trusted device, or an error if the proof was wrong or
/// the user declined. The phone is told the outcome either way.
pub async fn respond<F, Fut>(
    conn: &PeerConnection,
    manager: &PairingManager,
    our_hello: &Hello,
    approve: F,
) -> Result<TrustedDevice>
where
    F: FnOnce(PairingRequest) -> Fut,
    Fut: Future<Output = bool>,
{
    let (mut send, mut recv) = conn.accept_bi().await?;
    let their_hello = protocol::read_hello(&mut recv).await?;
    protocol::write_message(&mut send, &Message::Hello(our_hello.clone())).await?;
    protocol::negotiate(our_hello, &their_hello)?;

    let pair = match protocol::read_message(&mut recv).await? {
        Message::Pair(pair) => pair,
        other => bail!("expected PAIR, got {other:?}"),
    };
    let exporter = conn.export_keying_material(EXPORTER_LABEL, b"")?;

    if !manager.verify_and_consume(&pair.proof, &exporter, conn.peer_id()) {
        send_result(&mut send, Some("invalid or expired pairing code")).await?;
        bail!(
            "pairing proof from {} did not match an open session",
            conn.peer_id()
        );
    }

    let request = PairingRequest {
        device_id: *conn.peer_id(),
        device_name: their_hello.device_name,
        platform: their_hello.platform,
    };
    if !approve(request.clone()).await {
        send_result(&mut send, Some("declined on the other device")).await?;
        bail!("user declined pairing with {}", request.device_name);
    }

    let device = TrustedDevice {
        id: request.device_id,
        name: request.device_name,
        platform: request.platform,
    };
    manager.trust.add(device.clone());
    send_result(&mut send, None).await?;
    Ok(device)
}

/// Send `PAIR_RESULT` (refused if `refusal` is set) and wait until the phone
/// has it, so the caller can't drop the connection while it's in flight
async fn send_result(send: &mut quinn::SendStream, refusal: Option<&str>) -> Result<()> {
    let result = Message::PairResult(PairResult {
        accepted: refusal.is_none(),
        reason: refusal.map(Into::into),
    });
    protocol::finish_with(send, &result).await
}

/// Phone: pair with the desktop described by a scanned QR code. On success
/// the desktop is added to `trust` and the open connection is returned.
pub async fn initiate(
    endpoint: &QuicEndpoint,
    code: &PairingCode,
    our_hello: &Hello,
    our_id: &DeviceId,
    trust: &dyn TrustStore,
) -> Result<(PeerConnection, TrustedDevice)> {
    code.check_not_expired()?;
    let conn = endpoint
        .connect_any(&code.endpoints, &code.desktop_key)
        .await?;

    let their_hello =
        match exchange(&conn, code, our_hello, our_id).await {
            Ok(hello) => hello,
            // The desktop's authorizer turned us away: no open pairing session
            Err(e) if conn.was_refused() => return Err(e.context(
                "the other device refused the connection: this pairing code was already used or \
                 has expired. Ask for a new code.",
            )),
            Err(e) => return Err(e),
        };

    let device = TrustedDevice {
        id: *conn.peer_id(),
        name: their_hello.device_name,
        platform: their_hello.platform,
    };
    trust.add(device.clone());
    Ok((conn, device))
}

/// Phone side of the HELLO / PAIR / PAIR_RESULT exchange; returns the desktop's HELLO
async fn exchange(
    conn: &PeerConnection,
    code: &PairingCode,
    our_hello: &Hello,
    our_id: &DeviceId,
) -> Result<Hello> {
    let (mut send, mut recv) = conn.open_bi().await?;
    protocol::write_message(&mut send, &Message::Hello(our_hello.clone())).await?;
    let their_hello = protocol::read_hello(&mut recv).await?;
    protocol::negotiate(our_hello, &their_hello)?;

    let exporter = conn.export_keying_material(EXPORTER_LABEL, b"")?;
    let proof = pairing_proof(&code.secret, &exporter, our_id, conn.peer_id());
    let pair = Message::Pair(Pair {
        proof: *proof.as_bytes(),
    });
    protocol::write_message(&mut send, &pair).await?;
    send.finish()?;

    match protocol::read_message(&mut recv).await? {
        Message::PairResult(PairResult { accepted: true, .. }) => {}
        Message::PairResult(PairResult { reason, .. }) => {
            bail!(
                "pairing refused: {}",
                reason.unwrap_or_else(|| "no reason given".into())
            )
        }
        other => bail!("expected PAIR_RESULT, got {other:?}"),
    }
    Ok(their_hello)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceIdentity;
    use crate::trust::MemoryTrustStore;
    use std::net::{IpAddr, Ipv4Addr};

    const LOCALHOST: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

    struct Desktop {
        identity: Arc<DeviceIdentity>,
        manager: Arc<PairingManager>,
        endpoint: QuicEndpoint,
    }

    fn desktop() -> Desktop {
        let identity = Arc::new(DeviceIdentity::generate_new());
        let manager = Arc::new(PairingManager::new(
            identity.public_key(),
            Arc::new(MemoryTrustStore::new()),
        ));
        let endpoint = QuicEndpoint::bind(identity.clone(), LOCALHOST, manager.clone()).unwrap();
        Desktop {
            identity,
            manager,
            endpoint,
        }
    }

    struct Phone {
        identity: Arc<DeviceIdentity>,
        trust: MemoryTrustStore,
        endpoint: QuicEndpoint,
    }

    fn phone() -> Phone {
        let identity = Arc::new(DeviceIdentity::generate_new());
        let endpoint =
            QuicEndpoint::bind(identity.clone(), LOCALHOST, Arc::new(|_: &DeviceId| false))
                .unwrap();
        Phone {
            identity,
            trust: MemoryTrustStore::new(),
            endpoint,
        }
    }

    impl Phone {
        async fn pair(&self, code: &PairingCode) -> Result<TrustedDevice> {
            let hello = Hello::new("Test Phone");
            initiate(
                &self.endpoint,
                code,
                &hello,
                &self.identity.device_id(),
                &self.trust,
            )
            .await
            .map(|(_, device)| device)
        }
    }

    /// Desktop accepts one connection and runs `respond` with a fixed answer
    fn serve_once(
        desktop: Desktop,
        allow: bool,
    ) -> tokio::task::JoinHandle<(Result<TrustedDevice>, Desktop)> {
        tokio::spawn(async move {
            let result = async {
                let conn = desktop.endpoint.accept().await.unwrap()?;
                let hello = Hello::new("Test Desktop");
                respond(&conn, &desktop.manager, &hello, |_| async move { allow }).await
            }
            .await;
            (result, desktop)
        })
    }

    #[tokio::test]
    async fn successful_pairing_makes_both_sides_trust_each_other() {
        let desktop = desktop();
        let phone = phone();
        let code = desktop
            .manager
            .start_session(vec![desktop.endpoint.local_addr().unwrap()]);
        let desktop_id = desktop.identity.device_id();

        // Through the QR URI, as a real phone would receive it
        let scanned = PairingCode::from_uri(&code.to_uri().unwrap()).unwrap();
        let server = serve_once(desktop, true);
        let paired_desktop = phone.pair(&scanned).await.unwrap();
        let (paired_phone, desktop) = server.await.unwrap();
        let paired_phone = paired_phone.unwrap();

        assert_eq!(paired_desktop.id, desktop_id);
        assert_eq!(paired_desktop.name, "Test Desktop");
        assert!(phone.trust.is_trusted(&desktop_id));

        assert_eq!(paired_phone.id, phone.identity.device_id());
        assert_eq!(paired_phone.name, "Test Phone");
        assert!(desktop
            .manager
            .trust()
            .is_trusted(&phone.identity.device_id()));

        // Session was single use, so pairing is closed again
        assert!(!desktop.manager.is_pairing_open());
    }

    #[tokio::test]
    async fn declined_pairing_trusts_nobody() {
        let desktop = desktop();
        let phone = phone();
        let code = desktop
            .manager
            .start_session(vec![desktop.endpoint.local_addr().unwrap()]);

        let server = serve_once(desktop, false);
        let err = phone.pair(&code).await.unwrap_err();
        let (result, desktop) = server.await.unwrap();

        assert!(err.to_string().contains("declined"), "{err}");
        assert!(result.is_err());
        assert!(phone.trust.list().is_empty());
        assert!(desktop.manager.trust().list().is_empty());
    }

    #[tokio::test]
    async fn wrong_secret_is_refused_without_asking_the_user() {
        let desktop = desktop();
        let phone = phone();
        let mut code = desktop
            .manager
            .start_session(vec![desktop.endpoint.local_addr().unwrap()]);
        code.secret[0] ^= 1;

        let asked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let asked_in_task = asked.clone();
        let server = tokio::spawn(async move {
            let conn = desktop.endpoint.accept().await.unwrap().unwrap();
            let hello = Hello::new("Test Desktop");
            let result = respond(&conn, &desktop.manager, &hello, |_| async move {
                asked_in_task.store(true, std::sync::atomic::Ordering::SeqCst);
                true
            })
            .await;
            (result, desktop)
        });

        let err = phone.pair(&code).await.unwrap_err();
        let (result, desktop) = server.await.unwrap();

        assert!(err.to_string().contains("invalid"), "{err}");
        assert!(result.is_err());
        assert!(!asked.load(std::sync::atomic::Ordering::SeqCst));
        assert!(desktop.manager.trust().list().is_empty());
    }

    #[tokio::test]
    async fn code_cannot_be_used_twice() {
        let desktop = desktop();
        let first = phone();
        let second = phone();
        let code = desktop
            .manager
            .start_session(vec![desktop.endpoint.local_addr().unwrap()]);

        let server = serve_once(desktop, true);
        first.pair(&code).await.unwrap();
        let (_, desktop) = server.await.unwrap();

        // Session consumed: the endpoint now refuses unknown devices outright
        let server = serve_once(desktop, true);
        let err = second.pair(&code).await.unwrap_err();
        assert!(
            err.to_string().contains("already used or has expired"),
            "{err:#}"
        );
        let (result, _) = server.await.unwrap();
        assert!(result.is_err());
        assert!(second.trust.list().is_empty());
    }

    #[test]
    fn proof_is_bound_to_the_tls_session() {
        let desktop = DeviceIdentity::generate_new();
        let phone = DeviceIdentity::generate_new().device_id();
        let manager = PairingManager::new(desktop.public_key(), Arc::new(MemoryTrustStore::new()));
        let code = manager.start_session(vec![LOCALHOST]);

        // A proof computed on one connection is useless on another
        let proof = pairing_proof(&code.secret, &[1; 32], &phone, &desktop.device_id());
        assert!(!manager.verify_and_consume(proof.as_bytes(), &[2; 32], &phone));
        // ...and so is a proof for a different phone key
        let other_phone = DeviceIdentity::generate_new().device_id();
        assert!(!manager.verify_and_consume(proof.as_bytes(), &[1; 32], &other_phone));
        // The genuine one still works
        assert!(manager.verify_and_consume(proof.as_bytes(), &[1; 32], &phone));
    }

    #[test]
    fn session_closes_after_three_failed_attempts() {
        let desktop = DeviceIdentity::generate_new();
        let phone = DeviceIdentity::generate_new().device_id();
        let manager = PairingManager::new(desktop.public_key(), Arc::new(MemoryTrustStore::new()));
        let code = manager.start_session(vec![LOCALHOST]);

        for _ in 0..MAX_FAILED_ATTEMPTS {
            assert!(manager.is_pairing_open());
            assert!(!manager.verify_and_consume(&[0; 32], &[1; 32], &phone));
        }
        assert!(!manager.is_pairing_open());

        // Even the right proof is now refused
        let proof = pairing_proof(&code.secret, &[1; 32], &phone, &desktop.device_id());
        assert!(!manager.verify_and_consume(proof.as_bytes(), &[1; 32], &phone));
    }

    #[test]
    fn expired_session_is_refused() {
        let desktop = DeviceIdentity::generate_new();
        let phone = DeviceIdentity::generate_new().device_id();
        let manager = PairingManager::new(desktop.public_key(), Arc::new(MemoryTrustStore::new()));
        let mut code = manager.start_session_with_lifetime(vec![LOCALHOST], Duration::ZERO);
        // Force expiry into the past
        manager.sessions.lock().unwrap()[0].expires_at -= 1;
        code.expires_at -= 1;

        let proof = pairing_proof(&code.secret, &[1; 32], &phone, &desktop.device_id());
        assert!(!manager.verify_and_consume(proof.as_bytes(), &[1; 32], &phone));
        assert!(!manager.is_pairing_open());
    }

    #[test]
    fn authorizer_admits_unknown_devices_only_while_pairing() {
        let desktop = DeviceIdentity::generate_new();
        let trust = Arc::new(MemoryTrustStore::new());
        let manager = PairingManager::new(desktop.public_key(), trust.clone());
        let stranger = DeviceIdentity::generate_new().device_id();
        let friend = DeviceIdentity::generate_new().device_id();
        trust.add(TrustedDevice {
            id: friend,
            name: "Friend".into(),
            platform: "macos".into(),
        });

        assert!(!manager.authorize(&stranger));
        assert!(manager.authorize(&friend));
        manager.start_session(vec![LOCALHOST]);
        assert!(manager.authorize(&stranger));
    }

    #[test]
    fn uri_round_trips_and_stays_small() {
        let desktop = DeviceIdentity::generate_new();
        let code = PairingCode {
            desktop_key: desktop.public_key(),
            secret: [9; 32],
            expires_at: 1_790_000_000,
            endpoints: vec![
                "192.168.1.20:48210".parse().unwrap(),
                "10.0.0.7:48210".parse().unwrap(),
            ],
        };
        let uri = code.to_uri().unwrap();

        assert!(uri.starts_with("tovi://pair/"));
        assert!(uri.len() < 200, "QR URI is {} chars", uri.len());
        assert_eq!(PairingCode::from_uri(&uri).unwrap(), code);
    }

    #[test]
    fn malformed_uris_are_rejected() {
        assert!(PairingCode::from_uri("https://example.com").is_err());
        assert!(PairingCode::from_uri("tovi://pair/!!!").is_err());
        assert!(PairingCode::from_uri("tovi://pair/").is_err());

        // Valid CBOR but a future version
        let mut cbor = Vec::new();
        let wire = WireCode {
            v: 2,
            k: DeviceIdentity::generate_new().public_key().to_bytes(),
            s: [0; 32],
            x: 0,
            e: vec!["127.0.0.1:1".into()],
        };
        ciborium::into_writer(&wire, &mut cbor).unwrap();
        let uri = format!("{URI_PREFIX}{}", URL_SAFE_NO_PAD.encode(cbor));
        assert!(PairingCode::from_uri(&uri)
            .unwrap_err()
            .to_string()
            .contains("version"));
    }

    #[test]
    fn phone_rejects_long_expired_code_early() {
        let code = PairingCode {
            desktop_key: DeviceIdentity::generate_new().public_key(),
            secret: [0; 32],
            expires_at: unix_now() - CLOCK_SKEW_ALLOWANCE.as_secs() - 1,
            endpoints: vec![LOCALHOST],
        };
        assert!(code.check_not_expired().is_err());

        let just_expired = PairingCode {
            expires_at: unix_now() - 1,
            ..code
        };
        assert!(just_expired.check_not_expired().is_ok());
    }
}
