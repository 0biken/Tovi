# Android: background sends, Share → TOVI, discovery, permissions — design

Project 2 of 5 on the path to the first milestone (`docs/TASKS.md`: a 2 GB file
Android → Windows/macOS over home Wi‑Fi, paired by QR, no internet, survives a
Wi‑Fi drop). Covers TASKS §11: "NSD discovery adapter + MulticastLock",
"Share-sheet intent", "Foreground service for active transfers; notifications",
"Permissions per API level". Builds on `feat/discovery` (PR #14).

## Why

- Today a send runs inside the app process with nothing keeping it alive. Once the
  user switches apps, Android may kill a long send. A 2 GB send takes minutes, so
  the milestone can't be met reliably.
- Users should be able to send from any app ("Share → TOVI", Tech doc §32) rather
  than open TOVI and pick a file each time.
- The phone starts nodes with discovery off (`tovi-ffi` leaves
  `NodeConfig.discovery` as `None`). So a desktop that got a new address can only
  be reached again by re-pairing.

## Goals

- A send the user starts keeps running when they leave the app or turn the screen
  off, with a notification showing progress, until it finishes, fails, or gives up
  reconnecting.
- When a send finishes or fails while the app isn't visible, the user is told by a
  notification.
- Share one or more files from any app to TOVI, pick a paired device, and they send
  one after another.
- The phone finds a paired desktop again after its address changes, the same way the
  desktop and CLI already do.
- Correct permissions and manifest entries on Android 8 (API 26) to 16 (API 36).

## Non-goals

- Receiving while the app is closed (user choice: send only). Receiving keeps
  working while TOVI is open, as today.
- Pause and cancel (a later core project). The notification has no Cancel action yet.
- Wi‑Fi Direct / Aware, so no `NEARBY_WIFI_DEVICES`. The TASKS item that mentions it
  is reworded instead.
- Sending without copying to the cache first (see Constraints).
- iOS.

## Anti-goals

- A service or notification that stays after the last send ends.
- Holding the multicast lock while the app is in the background with nothing
  sending (battery).
- Silent failure. A send that ends badly while the app is hidden must leave a
  notification.

## Constraints

- minSdk 26, targetSdk/compileSdk 36. Kotlin + Compose, Navigation 3, the existing
  `NodeRepository` as the one owner of the `ToviNode`.
- Android 14+ requires a declared foreground-service type and its permission.
  Android 15 limits the `dataSync` type to 6 hours in 24, which is far beyond a
  2 GB LAN send. On reaching the limit, the system calls `Service.onTimeout`. Stop
  the service there; the send itself is reported as interrupted by the core.
- Android 13+ needs the runtime `POST_NOTIFICATIONS` permission. If the user
  refuses, sends still run in the foreground service (Android allows that), but
  the user sees no notification. In that case the app shows an in-app hint once.
- **Picked and shared files are still copied into the app cache before sending**,
  because the core reads by path (existing `OutgoingFiles`). For 2 GB this needs
  2 GB free and takes some seconds. The copy is deleted when the send ends, so a
  send that gives up after the 2-minute reconnect window can't be resumed across
  an app restart. Accepted for now: within the reconnect window, resume works.
  Changing this is a later item.
- **Build environment:** this PC has the Android SDK 36, NDK 29, the Rust Android
  targets and cargo-ndk. JDK 17 is being installed (Microsoft OpenJDK 17, via
  winget, at the user's go-ahead). No phone is attached. The
  emulator sits behind NAT, so it can't reach the desktop. On-device checks need
  the user's phone.

## Approaches considered (user picked both)

**Keeping a send alive**
1. **A foreground service of type `dataSync` (picked).** It starts with the first
   send and stops after the last. Same code on Android 8–16.
2. Android 14 user-initiated data transfer jobs, plus the service above for 8–13.
   Rejected: two code paths and an extra revocable permission.

**Discovery on the phone**
1. **The Rust mDNS discovery from PR #14, plus Android's multicast lock (picked).**
   `tovi-ffi` switches discovery on. Kotlin holds a `WifiManager.MulticastLock`
   while the app is visible or a send is running. Risk: `mdns-sd` hasn't been run on
   Android, so an early on-device check comes first. If it fails, fall back to
   approach 2. That would be a re-decision brought back to the user.
2. `NsdManager` behind the `Discovery` trait through a UniFFI callback interface.
   Rejected for now: a new Kotlin↔Rust layer, resolve quirks before API 34, and
   about twice the work.

## Design

**Rust (`core/tovi-ffi`)**
- `NodeConfig` gains `discovery: bool` (UniFFI default `false`). When it's true,
  start the node with `MdnsDiscovery::new()`. If that fails, log a warning and run
  without discovery, as the CLI does. Advertising stays at the core default (on),
  so the desktop can find the phone again while TOVI is open.
- No other API change.

**Keeping sends alive (`TransferService`)**
- A `Service` declared with `foregroundServiceType="dataSync"`, not exported.
- **One send queue (my call).** `NodeRepository` gets a FIFO queue that every send
  goes through: the file picker and Share → TOVI alike. A single worker coroutine
  takes one item at a time.
  - For each item: copy it to the cache, call `sendFile`, delete the copy, then
    take the next item.
  - A failed item doesn't stop the rest of the queue.
  - Only one file is copied at a time, so three shared 2 GB files need about 2 GB of
    free cache, not 6 GB.
  - The queue exposes a `StateFlow` of `{ current, remaining, finished results }`.
    This drives the notification and the "N files left" count.
- **Read access for shared files (my call).** The share grant must last until a file
  is copied, which may be minutes after `ShareActivity` closes. `ShareActivity`
  passes the URIs to `TransferService` in its start intent, as `ClipData` with
  `FLAG_GRANT_READ_URI_PERMISSION`, so the grant moves to the service. The service
  then enqueues them. A URI that can no longer be read when its turn comes fails
  that one item with "Could not read …".
- `NodeRepository` starts the service (`ContextCompat.startForegroundService`) when
  the queue goes from empty to non-empty. That covers the copy phase too.
- The service calls `startForeground` with a progress notification at once. Android
  requires this within a few seconds.
- The service observes the repository's in-flight sends. It counts sends from the
  moment one is requested, not from the `TransferStarted` event, so the copy phase
  is covered too. It updates one ongoing notification: the file name, the device,
  the percent, "Reconnecting…" when reconnecting, and "N files left" for a queue.
- When no sends remain, it calls `stopForeground(REMOVE)` and `stopSelf()`.
- The service owns no node logic. The node and its sends stay in `NodeRepository`
  on the process-wide `appScope`. The service exists only to keep the process in
  the foreground state while sends run.
- **Result notifications (my call):**
  - Post one summary when the queue empties, not one per file: "Sent 3 files to
    Desktop", or "Sent 2 of 3 to Desktop · couldn't send photo.jpg: reason". A single
    file reads "Sent X to Desktop" or "Couldn't send X: reason".
  - Post it only when TOVI isn't visible: `ProcessLifecycleOwner` is not started.
    That's the same process-wide signal the multicast lock uses, so `ShareActivity`
    closing counts as not visible.
  - When TOVI is visible, the existing snackbar is enough.
  - Tapping the notification opens Transfers.
- **Notification channels:** "Transfers" (low importance, ongoing progress) and
  "Transfer results" (default importance).

**Share → TOVI**
- A new `ShareActivity`, not the main one, so sharing doesn't disturb the user's
  navigation in TOVI:
  - It has intent filters for `ACTION_SEND` and `ACTION_SEND_MULTIPLE` with
    `*/*`.
  - It shows a "Send N files to…" list of paired devices, built from
    `repository.devices`.
  - Tapping a device calls `repository.send(deviceId, uri)` for each URI, then
    finishes the activity.
  - If there are no paired devices, it shows "Pair a device in TOVI first", with
    a button that opens the main app.
  - Text shares (`text/plain` with no stream) are out of scope. Only URIs from
    `EXTRA_STREAM` count. Read permission comes from the share intent's grant, so
    copying happens before the activity finishes, or the grant is carried over.
- Several files send one after another, in the order shared.

**Discovery and the multicast lock**
- `NodeRepository` passes `discovery = true`.
- A small `MulticastLockHolder` acquires the lock when the app comes to the
  foreground (`ProcessLifecycleOwner` started) or a send starts. It releases the
  lock when both are over. It uses a non-reference-counted lock, since the holder
  tracks both conditions itself.

**Permissions and manifest**
- Add `POST_NOTIFICATIONS`, `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_DATA_SYNC`
  and `CHANGE_WIFI_MULTICAST_STATE`.
- Ask for notifications on Android 13+ the first time the user sends, from
  whichever activity starts the send (`MainActivity` or `ShareActivity`), before
  enqueueing. A denial doesn't block the send (my call).
- If notifications are off, show a one-line hint the next time `MainActivity` opens.
  Show it once.
- Docs: tick the four TASKS §11 items, reword the `NEARBY_WIFI_DEVICES` item, and
  update `apps/android/README.md`.

## Order of work

1. **The `tovi-ffi` discovery flag, the multicast lock holder, and permissions.**
   Then an on-device check of Rust mDNS, run by the user on their phone.
   - Pass: the phone is paired with the desktop app; the desktop restarts TOVI on
     another port; a send from the phone reaches it; Devices then shows the new
     port.
   - The check gates only `discovery = true` and the multicast lock. Steps 2–4
     don't depend on it and go ahead regardless.
   - If it fails, `discovery` stays `false` in the app, and the NsdManager fallback
     is brought back to the user as its own decision.
2. The send queue, then `TransferService` with progress and result notifications.
3. `ShareActivity`.
4. Docs.

## Testing

- **Rust:** a `tovi-ffi` host test that `discovery: true` starts and shuts down
  cleanly. It must not need multicast to pass.
- **JVM unit tests** (existing pattern, `LiveStateTest`), for the pure parts:
  - the notification text from the live state;
  - the queue count;
  - the share-intent parsing (single, multiple, no stream);
  - the lock holder's acquire/release logic, with a fake lock.
- **On the user's phone:**
  1. Send 2 GB to the desktop with the screen off and the app swiped away, and
     confirm it completes.
  2. Toggle Wi‑Fi mid-send and confirm it resumes.
  3. Share 3 files from the Files app and confirm they send in order.
  4. Restart the desktop's TOVI on another port, then send, and confirm the phone
     finds it through discovery.
  5. Deny notifications and confirm the send still completes.

## Left to the builder

- The notification layout and icon.
- How the service observes in-flight sends: a `StateFlow` from the repository is
  the expected shape.
- The exact UI of the share picker, matching the existing Compose screens.
- Test helpers.
