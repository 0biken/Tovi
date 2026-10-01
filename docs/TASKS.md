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
- [x] **(D2)** Pairing: key pin + BLAKE3 keyed MAC over TLS exporter, then desktop approval
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

- [x] Transport abstraction: one secure QUIC endpoint (`QuicEndpoint`); LAN, Wi-Fi Direct/Aware and relay links supply IP paths to it (matches Tech doc §2 diagram)
- [x] QUIC LAN implementation with `quinn` (server + client endpoints, mutual TLS, ALPN `tvp/1`)
- [x] Server rejects client keys the `PeerAuthorizer` refuses (trust store + pairing sessions implement it in §5)
- [x] Enumerate candidate addresses (skip loopback, link-local, Hyper-V/WSL/Docker/VPN); IPv4
- [ ] IPv6 candidates and dual-stack bind
- [x] `connect_any`: try all QR endpoints in parallel, first verified connection wins
- [ ] Accept handshakes concurrently so slow or hostile clients cannot delay others
- [x] Connection timeouts (10 s connect, 30 s idle), 10 s keep-alive, graceful close with close codes

## 4. Core — discovery (`discovery`)

- [ ] `Discovery` trait (advertise / browse / events); call `ServiceDaemon::shutdown()` on drop
- [ ] Desktop implementation using `mdns-sd`, service `_tovi._udp.local`
- [ ] TXT record: protocol version, port, opaque ID (not the human device name)
- [ ] Bounded browse windows (battery, §38)

## 5. Core — pairing (new `pairing` module)

- [x] Ephemeral pairing session: 32-byte secret, absolute 60 s expiry, single use, closed after 3 failed attempts
- [x] QR payload encode/decode (`tovi://pair/<base64url(CBOR)>`, ~150 chars)
- [x] Terminal QR render for the CLI spike (§10)
- [x] Pairing handshake over QUIC, bound to TLS session (BLAKE3 keyed MAC over TLS exporter + both keys)
- [x] Desktop-side approval step (Allow / Cancel) required even with valid QR
- [x] Trusted-device store (`TrustStore` trait, in-memory)
- [ ] Persist trusted devices to disk (with §9 storage)
- [x] Revoke / forget device (`TrustStore::remove`)

## 6. Core — protocol (`protocol`)

- [x] Message types for connection and pairing: HELLO (incl. capabilities), PAIR, PAIR_RESULT
- [x] Transfer message types: TRANSFER_OFFER / TRANSFER_RESPONSE / CHUNK / TRANSFER_COMPLETE / TRANSFER_RESULT
- [x] Framing codec over QUIC streams (4-byte length + CBOR, 64 KiB cap, validated fields)
- [x] Version negotiation (`protocol`, `min_protocol`, capabilities)
- [x] Randomized decoder robustness test (20k random inputs, no panics)
- [ ] `cargo-fuzz` target for the decoder (malformed packet testing, §42)

## 7. Core — transfer engine (`transfer`)

- [x] Streaming chunk reader (no full-file buffering; at most 4 chunks in memory)
- [x] Per-chunk BLAKE3 hash + whole-file BLAKE3, receiver re-reads the file from disk to verify (`sha2` removed)
- [x] Receiver writes to `<name>.<id>.tovi.part`, verifies, then renames; part file deleted on failure
- [x] Filename sanitisation: strip separators and `..`, Windows reserved names and characters, length limits
- [x] Duplicate filename handling (`name (1).ext`)
- [x] Receive-folder setting (default `Downloads/TOVI`, CLI `--receive-dir`)
- [x] Low-disk-space check before accepting (file size + 64 MB margin)
- [ ] Transfer state machine (§23)
- [x] Progress events for UI (CLI shows percentage and MB/s)
- [ ] Pause / cancel

## 8. Core — resume

- [x] Receiver state (received chunk ranges) persisted next to the part file (`.tovi.state`), replaced atomically after every verified chunk
- [x] Resume negotiation: sender re-offers the same transfer ID; receiver replies with the chunk ranges it holds; sender sends only the rest
- [x] Source change detection (size + mtime) before resuming; whole-file hash still checked at the end
- [x] Auto-reconnect after network drop (`send_with_resume`: reconnects to the QR addresses with backoff for up to 2 minutes)
- [x] A newer connection takes over a transfer from a stalled one that has not noticed the drop yet
- [x] Dropped connection keeps partial files; corruption, decline or protocol errors delete them
- [x] Re-offer of an already-saved transfer is confirmed by its hash, not saved twice (remembered in memory for 10 minutes)
- [ ] Clean up abandoned partial files (e.g. older than 7 days)
- [ ] Resume across a sender app restart (persist outgoing transfer IDs)

## 9. Core — storage (new `storage` module)

- [ ] SQLite schema: devices, transfers, transfer_chunks, settings, sessions
- [ ] Migrations
- [ ] Transfer history queries

## 10. Spike validation (Tech doc §49)

- [x] CLI: `tovi-cli id` / `listen` (QR + approval prompt) / `pair <link>`; persistent identity via `--data-dir`
- [x] Live pairing between two CLI processes over the Wi-Fi address; reused link refused with a clear message
- [ ] Pair two physical machines over home Wi-Fi with the CLI
- [x] CLI: `tovi-cli send <file> <link>` (pairs, then sends on the same connection); `listen` receives from trusted devices only
- [x] 2 GiB transfer between two CLI processes on one machine via the Wi-Fi address: 86.6 MB/s, SHA-256 of source and received file identical
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
