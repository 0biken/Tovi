# TOVI — Task List (draft)

Target for the first milestone (Tech doc §54): **a 2 GB file moves Android → macOS over a
home LAN, paired by QR scan, with no internet dependency, and survives a Wi-Fi drop.**

Legend: `[ ]` open · `[x]` done · **(D)** = decision needed before dependent work starts

---

## 0. Setup & housekeeping

- [x] Remove duplicate docs; move PRD / Tech doc into `docs/`
- [x] `git init`, `.gitignore`
- [x] Make `tovi-core` structurally compilable (module stubs, single rustls crypto provider)
- [ ] Install Rust toolchain (rustup, stable) and confirm `cargo check` / `cargo clippy` pass
- [x] Initial commit; create remote repository (github.com/0biken/Tovi)
- [x] Root Cargo workspace (`Cargo.toml` at repo root, `core/tovi-core` as member)
- [ ] CI: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` on Windows/macOS/Linux — workflow written, not yet green
- [x] Split `tovi-core` into a library + a separate `tovi-cli` spike binary (`core/tovi-cli`)

## 1. Decisions & doc fixes

- [ ] **(D)** Certificate verification model: pin peer Ed25519 key (custom rustls verifiers on both sides); never "accept any cert"
- [ ] **(D)** QR pairing handshake: how the QR secret is bound to the TLS session (TLS exporter + HMAC, or a PAKE such as SPAKE2)
- [ ] **(D)** QR payload format: absolute expiry, multiple candidate endpoints, cert fingerprint, compact encoding (CBOR/base45 vs JSON)
- [ ] **(D)** FFI strategy for Android/iOS (UniFFI recommended)
- [ ] **(D)** Chunk size (benchmark 4 / 8 / 16 MiB) and hash choice (SHA-256 vs BLAKE3)
- [ ] **(D)** Wire encoding for TVP/1 messages (e.g. length-prefixed postcard/CBOR)
- [ ] **(D)** Spike desktop target: macOS (per Tech doc) vs Windows (current dev machine)
- [ ] Reconcile PRD vs Tech doc: QR lifetime (60s vs 30s), resume priority (P0 vs P1), Linux phase (1.5 vs P2), Windows timing (MVP vs Sprint 5)
- [ ] Fix QR lifecycle ordering in Tech doc §10; drop QR from transport list in §18
- [ ] Clarify X25519 in §11 (TLS 1.3 already provides it) or state its separate purpose
- [ ] Write `docs/protocol/TVP-1.md` (message types, framing, versioning, error codes)
- [ ] Write `docs/security/threat-model.md` expanding Tech doc §43

## 2. Core — identity (`identity`)

- [ ] Generate / load Ed25519 device keypair
- [ ] Derive `device_id` from public key
- [ ] Build self-signed QUIC certificate from the identity key (`rcgen`)
- [ ] Custom rustls `ServerCertVerifier` / `ClientCertVerifier` that pin known peer keys
- [ ] `KeyStore` trait for OS keychains (file-backed dev implementation first)

## 3. Core — transport (`transport`)

- [ ] `Transport` trait (connect / accept / open stream) — transport-agnostic per §2
- [ ] QUIC LAN implementation with `quinn` (server + client endpoints, mutual TLS)
- [ ] Bind to all interfaces; enumerate candidate addresses (skip Hyper-V/WSL/Docker/VPN where possible)
- [ ] Connection timeouts, keep-alive, graceful close

## 4. Core — discovery (`discovery`)

- [ ] `Discovery` trait (advertise / browse / events)
- [ ] Desktop implementation using `mdns-sd`, service `_tovi._udp.local`
- [ ] TXT record: protocol version, port, opaque ID (not the human device name)
- [ ] Bounded browse windows (battery, §38)

## 5. Core — pairing (new `pairing` module)

- [ ] Ephemeral pairing session: secret, nonce, absolute expiry, single use
- [ ] QR payload encode/decode + terminal QR render for the CLI spike
- [ ] Pairing handshake over QUIC, bound to TLS session
- [ ] Desktop-side approval step (Allow / Cancel) required even with valid QR
- [ ] Persist trusted device (public key, name, platform, trusted flag)
- [ ] Revoke / forget device

## 6. Core — protocol (`protocol`)

- [ ] Message types: HELLO, CAPABILITIES, PAIR, AUTH, TRANSFER_INIT / META / CHUNK / ACK / COMPLETE / VERIFY
- [ ] Framing codec over QUIC streams
- [ ] Version negotiation (`protocol`, `min_protocol`, capabilities)
- [ ] Fuzz tests for the decoder (malformed packet testing, §42)

## 7. Core — transfer engine (`transfer`)

- [ ] Streaming chunk reader (no full-file buffering)
- [ ] Per-chunk hash + whole-file SHA-256
- [ ] Receiver writes to `*.tovi.part`, verifies, then atomic rename
- [ ] Filename sanitisation: strip separators and `..`, Windows reserved names, length limits
- [ ] Duplicate filename handling (`name (1).ext`)
- [ ] Receive-folder setting (default `Downloads/TOVI`)
- [ ] Low-disk-space check before accepting
- [ ] Transfer state machine (§23)
- [ ] Progress / speed events for UI
- [ ] Pause / cancel

## 8. Core — resume

- [ ] Receiver manifest (received chunk set) persisted to disk
- [ ] Resume negotiation: sender sends manifest request, receiver replies with missing chunks
- [ ] Source change detection (size + mtime, or hash) before resuming
- [ ] Auto-reconnect after network drop

## 9. Core — storage (new `storage` module)

- [ ] SQLite schema: devices, transfers, transfer_chunks, settings, sessions
- [ ] Migrations
- [ ] Transfer history queries

## 10. Spike validation (Tech doc §49)

- [ ] CLI: `tovi-cli listen` / `tovi-cli pair` / `tovi-cli send <file> <device>`
- [ ] Desktop ↔ desktop 2 GB transfer over LAN, checksum verified
- [ ] Transfer resumes after Wi-Fi toggle mid-transfer
- [ ] QR pairing works with mDNS blocked
- [ ] Works with internet disconnected
- [ ] Throughput benchmark vs raw network speed
- [ ] Record answers to the five spike questions in `docs/architecture/spike-results.md`

## 11. Android client (Sprint 3)

- [ ] UniFFI bindings for `tovi-core`
- [ ] Kotlin + Compose app skeleton
- [ ] NSD discovery adapter + `MulticastLock`
- [ ] QR scanner (CameraX + ML Kit)
- [ ] Share-sheet intent ("Share → TOVI")
- [ ] Foreground service for active transfers; notifications
- [ ] Permissions per API level (`NEARBY_WIFI_DEVICES` on 13+)
- [ ] Android → macOS 2 GB transfer (the milestone)

## 12. Desktop app (Sprint 2)

- [ ] Tauri 2 + React + TypeScript scaffold using `tovi-core`
- [ ] Device list, QR display with expiry countdown
- [ ] Incoming-transfer approval prompt
- [ ] Drag-and-drop send
- [ ] Transfer progress + history
- [ ] Settings: receive folder, trusted devices, auto-accept
- [ ] Background/tray mode

## 13. Hardening (Sprint 4)

- [ ] Adversarial network tests (§41): sleep/wake, IP change, VPN, firewall, client isolation
- [ ] Security tests (§42): MITM, QR replay/expiry, impersonation, path traversal, revoked device
- [ ] External security review (MVP release criterion #9)
- [ ] Windows firewall rule / first-run prompt handling

## 14. Later (post-MVP)

- [ ] Windows context-menu, startup service (Sprint 5)
- [ ] Linux packaging (.deb, AppImage), system service (Sprint 6)
- [ ] iOS: Bonjour/Network framework adapter, Local Network permission, share extension (Sprint 7)
- [ ] Wi-Fi Direct / Wi-Fi Aware transports
- [ ] Relay + NAT traversal
- [ ] Browser fallback (research: plain HTTP on LAN vs WebTransport `serverCertificateHashes`)
