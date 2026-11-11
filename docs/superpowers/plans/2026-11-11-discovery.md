# Discovery and Concurrent Handshakes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A send to a paired device whose address changed finds it again over mDNS without re-pairing, and a stalled handshake no longer blocks other connections.

**Architecture:** `QuicEndpoint` exposes the raw incoming connection so `Node`'s accept loop can finish each handshake in its own task, at most 32 at a time. A `Discovery` trait (mDNS implementation in `tovi-core`; Android NSD later) advertises anonymously and browses on demand. `Node` connects through one helper that tries the saved addresses and then discovery's addresses, pinning the device's key either way. The same helper serves reconnects inside `send_with_resume`, which now takes a connect closure instead of an address list.

**Tech Stack:** Rust (stable ≥ 1.85 for async closures; the repo uses 1.98/1.99), tokio, quinn 0.11, mdns-sd 0.11.

**Spec:** `docs/superpowers/specs/2026-11-11-discovery-design.md`

## Global Constraints

- Service type `_tovi._udp.local.` (existing `discovery::SERVICE_TYPE`).
- The TXT record holds only `v=1`. The instance name is the random per-run 16-hex-char ID (existing `random_instance_id`). No device name, platform or stable ID.
- Browse only on demand, `BROWSE_WINDOW = 3 s`. Never browse continuously.
- At most `MAX_CONCURRENT_HANDSHAKES = 32`. Past that, `incoming.refuse()`.
- IPv4 addresses only from discovery.
- `tovi-ffi` keeps `discovery: None`. No FFI changes in this plan.
- CI gates: `cargo fmt --all -- --check`; `cargo clippy --workspace --exclude tovi-desktop --all-targets --all-features -- -D warnings`; `cargo test --workspace --exclude tovi-desktop`; `cargo clippy -p tovi-desktop --all-targets -- -D warnings` (after `pnpm build` in `apps/desktop`). Tests must not need multicast; real mDNS tests are `#[ignore]`.

## Review Focus

1. **Discovery returns this device's own address** (it hears its own advert). The pinned key fails the TLS handshake, so the send still reaches the right device. Test: `send_skips_own_and_stranger_adverts` (Task 3).
2. **Discovery returns another TOVI device that isn't the target.** The pinned key rejects it and no file goes to the stranger. Test: same test, with a third node (Task 3).
3. **Discovery finds nothing.** The send fails with an `Interrupted` outcome and a message that says the device wasn't found on this network, not a panic or a hang past about 13 s. Test: `send_fails_cleanly_when_discovery_finds_nothing` (Task 3).
4. **Advertising fails** (no multicast, firewall). `Node::start` still succeeds and only logs a warning. Test: `start_survives_advertise_failure` (Task 3).
5. **Nodes without discovery** (Android today). Behavior is unchanged. Covered by every existing `node::tests` test, which uses `discovery: None`.

---

### Task 1: Concurrent handshakes

**Files:**
- Modify: `core/tovi-core/src/transport.rs:155-177` (accept split)
- Modify: `core/tovi-core/src/node.rs:171-187` (`Inner`), `:566-578` (`accept_loop`)
- Test: `core/tovi-core/src/node/tests.rs`

**Interfaces:**
- Produces: `QuicEndpoint::accept_incoming(&self) -> Option<quinn::Incoming>`; `pub async fn finish_accept(&self, incoming: quinn::Incoming) -> Result<PeerConnection>` (existing body, now `pub`); `QuicEndpoint::accept()` unchanged in behavior (`accept_incoming` + `finish_accept`).
- Produces: `pub const MAX_CONCURRENT_HANDSHAKES: usize = 32;` in `node.rs`.

- [ ] **Step 1: Write the failing test `stalled_handshake_does_not_block_others`** in `node/tests.rs`.
  1. Start `desktop` and `phone` and `pair(&mut desktop, &phone, true)`.
  2. Start a stall proxy: a `tokio::net::UdpSocket` on `127.0.0.1:0` that forwards **only the first datagram** it receives to `desktop.node.local_addr()` and drops everything after that, in both directions. Spawn a throwaway client: a `QuicEndpoint::bind` with a fresh `DeviceIdentity` that runs `connect(proxy_addr, &desktop_key)` and never completes. Wait 200 ms so desktop has taken that incoming.
  3. Assert: `tokio::time::timeout(Duration::from_secs(3), phone.node.send_file(&desktop_id, &path)).await` is `Ok(Ok(_))`, and the received file's bytes equal the source.

- [ ] **Step 2: Run it and verify it fails**
  Run: `cargo test -p tovi-core stalled_handshake_does_not_block_others`
  Expected: FAIL (timeout elapsed), because the accept loop is stuck for 10 s on the stalled handshake.

- [ ] **Step 3: Split accept in `transport.rs`.** Add `accept_incoming` (`self.endpoint.accept().await`), make `finish_accept` `pub`, and keep `accept()` as the wrapper.

- [ ] **Step 4: Rewrite `accept_loop` in `node.rs`.** Add `handshakes: Arc<tokio::sync::Semaphore>` to `Inner`, created with `MAX_CONCURRENT_HANDSHAKES` permits. For each `accept_incoming()`:
  - If `try_acquire_owned()` fails, `incoming.refuse()`, log at `warn`, and continue.
  - Otherwise spawn into `connection_tasks` one task that runs `finish_accept`. It drops the permit right after the handshake, before `handle_connection`, so long transfers don't hold permits. On `Ok(conn)` it calls `handle_connection(inner, conn)`. On `Err` it logs at `info` ("refused a connection"), the same as today.
  - Keep the `try_join_next` pruning.

- [ ] **Step 5: Run the test and the whole core suite**
  Run: `cargo test -p tovi-core`
  Expected: all pass, including `stalled_handshake_does_not_block_others`.

- [ ] **Step 6: Commit**
  ```bash
  git add core/tovi-core/src/transport.rs core/tovi-core/src/node.rs core/tovi-core/src/node/tests.rs
  git commit -m "Accept handshakes concurrently so a stalled client can't block others"
  ```

---

### Task 2: `Discovery` trait and `MdnsDiscovery`

**Files:**
- Modify (rewrite): `core/tovi-core/src/discovery.rs`

**Interfaces:**
- Produces:
  ```rust
  pub const BROWSE_WINDOW: Duration = Duration::from_secs(3);
  pub type BrowseFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>>> + Send + 'a>>;
  pub trait Discovery: Send + Sync + fmt::Debug {
      /// Start advertising this device at `port`; replaces any earlier advert
      fn advertise(&self, port: u16) -> Result<()>;
      fn stop_advertising(&self);
      /// TOVI services seen within `window`: IPv4, deduplicated, sorted
      fn browse(&self, window: Duration) -> BrowseFuture<'_>;
  }
  pub struct MdnsDiscovery { /* daemon, instance_id, registered fullname: Mutex<Option<String>> */ }
  impl MdnsDiscovery { pub fn new() -> Result<Self>; pub fn instance_id(&self) -> &str; }
  ```
  `DiscoveryManager` is removed (nothing uses it).

- [ ] **Step 1: Write the failing tests** in `discovery.rs`'s `mod tests`. Keep the two existing ID tests.
  - `advert_carries_only_the_version`: `service_info("0123456789abcdef", 48210)` (a private helper that builds the `ServiceInfo`). Assert that `info.get_properties().len() == 1`, `info.get_property_val_str("v") == Some("1")`, `info.get_port() == 48210`, and `info.get_fullname() == "0123456789abcdef._tovi._udp.local."`.
  - `browse_returns_within_window` (`#[ignore]`, needs multicast): `MdnsDiscovery::new()`, then `browse(Duration::from_millis(500))` completes in under 1.5 s.
  - `two_daemons_find_each_other` (`#[ignore]`): A advertises on port 50001. B's `browse(BROWSE_WINDOW)` contains a `SocketAddr` with port 50001.

- [ ] **Step 2: Run them and verify they fail**
  Run: `cargo test -p tovi-core discovery`
  Expected: compile error (`service_info` not defined).

- [ ] **Step 3: Implement `discovery.rs`.**
  - `service_info` uses host name `tovi-<id>.local.` and properties `{"v": "1"}` with `enable_addr_auto()`.
  - `advertise` unregisters any previous advert, registers the new one and stores its fullname. `stop_advertising` unregisters it and ignores errors.
  - `browse` calls `daemon.browse(SERVICE_TYPE)` and loops on `recv_async()` under `tokio::time::timeout_at(start + window)`. On each `ServiceResolved(info)` it skips its own fullname and collects `SocketAddr::new(ip, info.get_port())` for IPv4 `ip`s into a `BTreeSet`. Afterwards it calls `daemon.stop_browse(SERVICE_TYPE)` and returns the set as a `Vec`.
  - `impl Drop` calls `stop_advertising()` and then `daemon.shutdown()`, ignoring errors.

- [ ] **Step 4: Run the tests and verify they pass**
  Run: `cargo test -p tovi-core discovery` (non-ignored pass), then locally `cargo test -p tovi-core discovery -- --ignored`.
  Expected: PASS. The ignored ones pass on a machine with multicast. If the firewall blocks them, note it in the PR rather than failing the task.

- [ ] **Step 5: Commit**
  ```bash
  git add core/tovi-core/src/discovery.rs
  git commit -m "Add the Discovery trait and an mDNS implementation with an anonymous advert"
  ```

---

### Task 3: `Node` finds moved devices through discovery

**Files:**
- Modify: `core/tovi-core/src/transfer.rs:362-439` (`send_with_resume`, `reconnect`) and its two test call sites (`~1466`, `~1865`)
- Modify: `core/tovi-core/src/node.rs` (`NodeConfig`, `Inner`, `start`, `send_file`, `shutdown`)
- Test: `core/tovi-core/src/node/tests.rs`

**Interfaces:**
- Consumes: `Discovery`, `BROWSE_WINDOW` (Task 2).
- Produces:
  - `send_with_resume(conn: PeerConnection, transfer: &OutgoingTransfer, connect: impl AsyncFn() -> Result<PeerConnection>, progress: impl Fn(Progress), on_drop: impl Fn(&anyhow::Error)) -> Result<(PeerConnection, blake3::Hash)>`. The `endpoint`, `addrs` and `server_key` parameters go away, and `reconnect` takes `&connect` instead. The two transfer tests pass `async || endpoint.connect_any(&addrs, &key).await`.
  - `NodeConfig.discovery: Option<Arc<dyn Discovery>>` (default `None`) and `NodeConfig.advertise: bool` (default `true`; only matters when `discovery` is `Some`).
  - Private `Inner::connect_device(&self, device: &TrustedDevice) -> Result<PeerConnection>`.

- [ ] **Step 1: Write the failing tests** in `node/tests.rs`. Add a test-only `FakeDiscovery { found: Mutex<Vec<SocketAddr>>, advertise_fails: bool, advertised: Mutex<Option<u16>> }` implementing `Discovery`, and a `start_with(name, discovery)` helper.
  - `send_finds_moved_device`:
    1. Pair `phone` with `desktop`, then `shutdown` desktop and restart it with `start_in` on the same data dir. It gets a new port, so phone's saved address is stale.
    2. Phone's fake returns `[desktop.local_addr()]`.
    3. `send_file` succeeds, and phone's `store().device_addresses(&desktop_id)?[0] == desktop.local_addr()`.
  - `send_skips_own_and_stranger_adverts`: same stale setup, plus a third unpaired node `stranger`. The fake returns `[phone.local_addr(), stranger.local_addr(), desktop.local_addr()]`. The send succeeds, the file lands in desktop's inbox, and stranger's inbox is empty.
  - `send_fails_cleanly_when_discovery_finds_nothing`: stale setup, and the fake returns `[]`.
    - `send_file` returns `Err` within 15 s, and the error text contains `"not found on this network"`.
    - Phone sees `TransferFinished { outcome: Interrupted { .. } }`.
  - `start_survives_advertise_failure`: `advertise_fails: true`. `Node::start` returns `Ok`.
  - `listening_node_advertises_its_port`: the fake's `advertised` equals `Some(node.local_addr().port())`. After `shutdown`, it's `None`.

- [ ] **Step 2: Run them and verify they fail**
  Run: `cargo test -p tovi-core node::tests`
  Expected: compile error (`NodeConfig` has no field `discovery`).

- [ ] **Step 3: Change `send_with_resume` and `reconnect` in `transfer.rs`** to the connector signature above, and update the two test call sites. The behavior stays the same: backoff, deadline, and never retrying a refusal.

- [ ] **Step 4: Wire discovery into `node.rs`.**
  - **Config.** Add the two `NodeConfig` fields; `Option<Arc<dyn Discovery>>` is `Debug + Clone` because the trait requires `Debug`. `Inner` stores `discovery: Option<Arc<dyn Discovery>>`.
  - **Start.** In `start`, after binding: if `advertise` and `discovery` is set, call `discovery.advertise(local port)`. On `Err`, `tracing::warn!` and carry on.
  - **Shutdown.** In `shutdown`, call `stop_advertising()` first.
  - **`connect_device`.**
    1. Try `connect_any(saved addresses, &key)`. If it succeeds, return the connection.
    2. Otherwise, with no discovery, return that error with today's wording.
    3. With discovery, `browse(BROWSE_WINDOW)`, drop addresses already tried, and `connect_any` the rest with the same key. On success, `store.set_device_addresses(&device.id, [remote] + saved minus remote)`.
    4. On failure (or an empty browse), return `anyhow!("could not reach {name}: not found on this network; is TOVI running there?")` with the original error as context.
  - **`send_file`.** Allow empty saved addresses when discovery is set, so it can still browse. Use `connect_device` for the first connection, and `async || inner.connect_device(&device).await` as the `send_with_resume` connector.

- [ ] **Step 5: Run the whole suite**
  Run: `cargo test --workspace --exclude tovi-desktop`
  Expected: all pass, including the 5 new tests and the unchanged `tovi-ffi` tests.

- [ ] **Step 6: Commit**
  ```bash
  git add core/tovi-core/src/transfer.rs core/tovi-core/src/node.rs core/tovi-core/src/node/tests.rs
  git commit -m "Find a paired device again through discovery when its address changed"
  ```

---

### Task 4: Turn discovery on in the CLI and desktop app; update docs

**Files:**
- Modify: `core/tovi-cli/src/main.rs:136-150` (`node_config`, `outgoing_config`)
- Modify: `apps/desktop/src-tauri/src/lib.rs:27`
- Modify: `docs/TASKS.md` (§3, §4, §9), `README.md` ("automatic discovery is not built yet" note)

**Interfaces:**
- Consumes: `MdnsDiscovery::new()`, `NodeConfig.discovery`, `NodeConfig.advertise`.

- [ ] **Step 1: CLI.** `node_config` sets `discovery = MdnsDiscovery::new().ok().map(|d| Arc::new(d) as Arc<dyn Discovery>)`. If that fails, it logs a warning and runs without discovery. `outgoing_config` (for `pair` and `send`) also sets `advertise = false`, so short-lived senders don't advertise.

- [ ] **Step 2: Desktop.** Do the same in `apps/desktop/src-tauri/src/lib.rs` before `Node::start`, with `advertise` left `true`.

- [ ] **Step 3: Docs.**
  - `TASKS.md`: tick §3 "Accept handshakes concurrently…", all four §4 items, and §9 "Paired device whose address changed…" (reword it to "found again via discovery").
  - `README.md`: replace the note that ends "automatic discovery is not built yet" with one saying a moved device is found again on the same network. Mention that the Windows firewall prompt must allow private networks for this, too.

- [ ] **Step 4: Verify every CI gate**
  ```bash
  cargo fmt --all -- --check
  cargo clippy --workspace --exclude tovi-desktop --all-targets --all-features -- -D warnings
  cargo test --workspace --exclude tovi-desktop
  ```
  ```bash
  cd apps/desktop && pnpm build && cd ../.. && cargo clippy -p tovi-desktop --all-targets -- -D warnings
  ```
  Expected: no diff, no warnings, all tests pass.

- [ ] **Step 5: Manual check on one PC.**
  1. `cargo run -p tovi-cli -- --data-dir A listen --port 48300`, and pair B from another terminal: `cargo run -p tovi-cli -- --data-dir B pair "<link>"`.
  2. Stop A and restart it with `--port 48301`.
  3. `cargo run -p tovi-cli -- --data-dir B send README.md <A's name>` should succeed.
  4. `cargo run -p tovi-cli -- --data-dir B devices` should show port 48301.
  Record the outcome in the PR description.

- [ ] **Step 6: Commit, push, open the PR** (base `feat/desktop-polish` until #13 merges, then retarget to `main`)
  ```bash
  git add core/tovi-cli/src/main.rs apps/desktop/src-tauri/src/lib.rs docs/TASKS.md README.md
  git commit -m "Turn on discovery in the CLI and desktop app"
  git push -u origin feat/discovery
  ```
