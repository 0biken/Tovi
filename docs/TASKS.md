# TOVI — Task List (draft)

Target for the first milestone (Tech doc §54, amended by D7): **a 2 GB file moves Android → Windows and macOS over a
home LAN, paired by QR scan, with no internet dependency, and survives a Wi-Fi drop.**

Legend: `[ ]` open · `[x]` done · **(D)** = decision needed before dependent work starts

Decisions are recorded in [`architecture/decisions.md`](architecture/decisions.md) (D1–D7).

---

## 0. Setup & housekeeping

- [x] Remove duplicate docs; move PRD / Tech doc into `docs/`
- [x] `git init`, `.gitignore`
- [x] Make `tovi-core` structurally compilable (module stubs, single rustls crypto provider)
- [x] Install Rust toolchain (rustup, stable 1.98.1 + MSVC Build Tools); `cargo fmt` / `clippy` / `test` pass locally
- [x] Initial commit; create remote repository (github.com/0biken/Tovi)
- [x] Root Cargo workspace (`Cargo.toml` at repo root, `core/tovi-core` as member)
- [x] CI: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` on Windows/macOS/Linux (green on PR #1)
- [x] Split `tovi-core` into a library + a separate `tovi-cli` spike binary (`core/tovi-cli`)

## 1. Decisions & doc fixes

- [x] **(D1)** Certificate verification: pin peer Ed25519 key, mutual TLS, handshake signature always verified
- [x] **(D2)** Pairing: key pin + HMAC over TLS exporter, then desktop approval
- [x] **(D3)** QR payload: `tovi://pair/<base64url(CBOR)>`, absolute 60 s expiry, multiple endpoints
- [x] **(D4)** Mobile bridge: UniFFI, via a thin `tovi-ffi` crate
- [x] **(D5)** Hash: BLAKE3. Chunk size: 8 MiB provisional (benchmark in §10)
- [x] **(D6)** Wire encoding: length-prefixed CBOR (`ciborium`), 64 KiB control-frame cap, raw chunk bytes
- [x] **(D7)** Spike desktop target: Windows and macOS together
- [x] Reconcile PRD vs Tech doc: QR lifetime 60 s, resume P0, Linux phase 1.5, Windows in first milestone
- [x] Fix QR lifecycle ordering in Tech doc §10; drop QR from transport list in §18
- [x] Clarify X25519 in §11 (performed by TLS 1.3; no separate layer)
- [ ] Write `docs/protocol/TVP-1.md` (message types, framing, versioning, error codes)
- [ ] Write `docs/security/threat-model.md` expanding Tech doc §43

## 2. Core — identity (`identity`)

- [x] Generate / load Ed25519 device keypair
- [x] Derive `device_id` from public key
- [x] Build self-signed QUIC certificate from the identity key (`rcgen`)
- [x] Custom rustls `ServerCertVerifier` / `ClientCertVerifier` that pin known peer keys
- [x] `KeyStore` trait + file-backed dev implementation
- [ ] OS keychain `KeyStore` implementations (Windows DPAPI, macOS Keychain first per D7)

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
- [ ] Per-chunk BLAKE3 hash + whole-file BLAKE3 (replace `sha2` dependency)
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
- [ ] Android → Windows and Android → macOS 2 GB transfer (the milestone)

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
- [ ] Windows firewall rule / first-run prompt handling (needed for milestone, D7)

## 14. Later (post-MVP)

- [ ] Windows context-menu, startup service (Sprint 5)
- [ ] Linux packaging (.deb, AppImage), system service (Sprint 6)
- [ ] iOS: Bonjour/Network framework adapter, Local Network permission, share extension (Sprint 7)
- [ ] Wi-Fi Direct / Wi-Fi Aware transports
- [ ] Relay + NAT traversal
- [ ] Browser fallback (research: plain HTTP on LAN vs WebTransport `serverCertificateHashes`)
