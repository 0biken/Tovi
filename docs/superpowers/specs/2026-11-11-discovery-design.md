# Discovery and concurrent handshakes: design

Project 1 of 5 on the path to the first milestone (`docs/TASKS.md`: a 2 GB file
Android → Windows/macOS over home Wi‑Fi). Covers TASKS §3 "Accept handshakes
concurrently", all of §4, and §9 "Paired device whose address changed".

## Why

- A paired device is reached only at its saved addresses. When the router hands it a
  new address, sending fails until the devices pair again.
- `QuicEndpoint::accept` finishes each handshake (up to 10 s) before taking the next,
  so one slow or hostile client stalls everyone else.

## Goals

- A send to a paired device whose address changed succeeds without re-pairing, and the
  new address is saved.
- A stalled handshake doesn't delay other connections.
- Adverts reveal nothing personal and can't be used to track a device across runs.

## Non-goals

- Online status in the device list (desktop item, later).
- IPv6 (the transport is IPv4 only today).
- Android NSD adapter (project 2; the trait below is shaped for it).

## Anti-goals

- Continuous background browsing (battery, Tech doc §38).
- Any stable identifier, device name or platform in the advert.

## Approaches considered

1. **Try each advert (picked by the user).** Adverts stay anonymous: a random ID per
   run. When the saved addresses fail, browse briefly, connect to every TOVI service
   found, and keep the connection whose pinned key matches. No schema change; the same
   flow works for Android NSD. Cost: one handshake per TOVI device on the LAN, and
   strangers' devices see our public key during the TLS handshake.
2. Per-pair secret tags: each device advertises a rotating keyed hash only its paired
   devices recognise. Direct match, but needs a new secret stored at pairing (existing
   pairings must re-pair), bigger adverts and more Android work.
3. Stable device-ID hash: simplest, but makes devices trackable. Rejected.

## Design

**Concurrent accept.** `QuicEndpoint` gains `accept_incoming()` (the raw
`quinn::Incoming`) and a public `finish_accept()`. `Node`'s accept loop spawns
`finish_accept` + `handle_connection` per incoming into its existing task set. At most
32 handshakes run at once; past that, the incoming is refused rather than queued.
`QuicEndpoint::accept()` stays as a wrapper.

**Discovery.** A `Discovery` trait: `advertise(port)`, `stop_advertising()`, and an
async `browse(window) -> Vec<SocketAddr>` that is bounded and deduplicated.
`MdnsDiscovery` implements it with `mdns-sd`, service `_tovi._udp.local`. The TXT
record holds only `v=1`; the instance name is the random per-run ID. It calls
`ServiceDaemon::shutdown()` on drop. IPv4 only.

Browsing happens only on demand, with a 3 s window. A listening Node advertises the
whole time it runs.

**Node.** `NodeConfig` gets `discovery: Option<Arc<dyn Discovery>>`. The CLI and
desktop app pass `MdnsDiscovery`; `tovi-ffi` passes `None` for now. When connecting to
the saved addresses fails with a connect error (not a refusal), the Node browses and
tries `connect_any` on the found addresses with the device's pinned key. On success it
saves the new address. The same fallback runs in the reconnect loop of
`send_with_resume`.

## Left to the builder

Exact error classification for "connect error vs refusal" (reuse what the reconnect
loop already uses), test helpers, and log wording.
