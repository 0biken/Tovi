# Android Background Sends, Share and Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sends from the Android app keep running in a foreground service with progress and summary notifications, files can be shared to TOVI from any app, and the phone finds a moved desktop over mDNS.

**Architecture:**
- **Send queue.** `NodeRepository` funnels every send through one FIFO `SendQueue` that copies, sends and cleans up one item at a time.
- **Foreground service.** `TransferService` (type `dataSync`) runs while the queue is non-empty. It only shows notifications; the node stays in the repository.
- **Discovery.** `tovi-ffi` gains a `discovery` flag that turns on the Rust `MdnsDiscovery` from PR #14. Kotlin holds the Wi‑Fi multicast lock while the app is visible or a send is queued.
- **Sharing.** `ShareActivity` turns share intents into queue items, handing URI read grants to the service.

**Tech Stack:** Kotlin 2.3, Jetpack Compose (BOM 2026.03.01), AndroidX Lifecycle 2.10.0, coroutines 1.10.2, JUnit 4; Rust + UniFFI 0.32 (`core/tovi-ffi`).

**Spec:** `docs/superpowers/specs/2026-11-12-android-background-design.md`

## Global Constraints

- minSdk 26, targetSdk/compileSdk 36. Package `app.tovi.android`, generated bindings in `app.tovi.core`.
- The foreground service type is `dataSync`. Permissions: `POST_NOTIFICATIONS`, `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_DATA_SYNC`, `CHANGE_WIFI_MULTICAST_STATE`. No `NEARBY_WIFI_DEVICES`.
- Receiving while the app is closed is out of scope. No pause or cancel. Text shares (no `EXTRA_STREAM`) are ignored.
- Notification channels: `transfers` ("Transfers", `IMPORTANCE_LOW`, ongoing progress) and `transfer_results` ("Transfer results", `IMPORTANCE_DEFAULT`).
- Summary copy, verbatim:
  - all sent: `Sent N files to <device>`, or for one file `Sent <file> to <device>`;
  - some failed: `Sent K of N to <device> · couldn't send <first failed file>: <its error>`;
  - one file failed: `Couldn't send <file>: <error>`.
- Post the summary only when the app isn't visible (`ProcessLifecycleOwner` not started). Tapping it opens `MainActivity`, which opens Transfers.
- A failed item never stops the rest of the queue. Only one item is copied to the cache at a time.
- No service or notification may stay after the queue empties. The multicast lock is held only while the app is visible or the queue is non-empty.
- If `MdnsDiscovery::new()` fails, run without discovery (warning only).
- Build gates:
  - Rust: `cargo fmt --all -- --check`; `cargo clippy --workspace --exclude tovi-desktop --all-targets --all-features -- -D warnings`; `cargo test --workspace --exclude tovi-desktop`.
  - Android, from `apps/android` with `JAVA_HOME` set to the Microsoft OpenJDK 17 folder under `C:\Program Files\Microsoft\`: `./gradlew testDebugUnitTest` and `./gradlew assembleDebug`.
- The commit trailer is `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never stage the untracked `.mp4` or `media/`.

## Review Focus

1. **A shared file can no longer be read when its turn comes.** That item fails with `Could not read <file>: …` and the queue goes on. Test: `unreadableItemFailsAndQueueContinues` (Task 2).
2. **A send is requested while the queue is busy.** It waits its turn rather than running in parallel, and the remaining count includes it. Test: `itemsRunOneAtATimeInOrder` (Task 2).
3. **The service starts before the node is ready, or during the cache copy.** The notification shows `Preparing <file>…` straight away, with no percent. Test: `preparingWhenNoLiveTransfer` (Task 3).
4. **The app is visible when the queue drains.** No summary notification; the snackbar covers it. Test: `noSummaryWhileVisible` (Task 3).
5. **The app goes to the background mid-send.** The multicast lock stays held until the queue empties, and is released only then. Test: `lockHeldUntilQueueEmptiesAfterBackgrounding` (Task 4).

---

### Task 1: Discovery flag in `tovi-ffi`

**Files:**
- Modify: `core/tovi-ffi/src/types.rs` (`NodeConfig`), `core/tovi-ffi/src/lib.rs` (`ToviNode::start`)
- Test: `core/tovi-ffi/tests/node.rs`

**Interfaces:**
- Produces: `NodeConfig.discovery: bool` with `#[uniffi(default = false)]`, appearing in Kotlin as `discovery: Boolean = false`.

- [ ] **Step 1: Write the failing test `discovery_node_starts_and_stops`.** Start a `ToviNode` with `listen: Some("127.0.0.1:0")` and `discovery: true`. Assert that `node.port()` is `Ok`, then call `rt.block_on(node.shutdown())` (or the file's existing shutdown helper) and assert it returns within 5 s. Add `discovery: false` to the existing `NodeConfig` literals.
- [ ] **Step 2: Run** `cargo test -p tovi-ffi discovery_node_starts_and_stops`. Expected: compile error, no field `discovery`.
- [ ] **Step 3: Implement.** Add the field with the doc comment "Find paired devices on the local network after their address changes (mDNS)". In `start`:
  - when the flag is set, assign `core_config.discovery = MdnsDiscovery::new().map(|d| Arc::new(d) as Arc<dyn Discovery>).map_err(|e| tracing::warn!(...)).ok()`;
  - otherwise leave it as it is.
- [ ] **Step 4: Run the Rust gates.** Expected: all pass, and the new test passes without multicast. It only constructs the daemon, so if the daemon can't start the node still runs.
- [ ] **Step 5: Commit** `core/tovi-ffi/src/types.rs core/tovi-ffi/src/lib.rs core/tovi-ffi/tests/node.rs` with the message "Let the mobile apps turn on discovery".

---

### Task 2: One send queue

**Files:**
- Create: `apps/android/app/src/main/java/app/tovi/android/data/SendQueue.kt`
- Modify: `apps/android/app/src/main/java/app/tovi/android/data/NodeRepository.kt` (`send` → enqueue), `apps/android/app/src/main/java/app/tovi/android/ui/AppViewModel.kt:101-109` (passes the device name)
- Test: `apps/android/app/src/test/java/app/tovi/android/data/SendQueueTest.kt`

**Interfaces:**
- Produces (pure Kotlin, no Android types, so it runs under the JVM tests):
  ```kotlin
  data class QueueItem(val deviceId: String, val deviceName: String, val source: String /* content URI string */, val fileName: String)
  data class ItemResult(val item: QueueItem, val error: String? /* null = sent */)
  data class QueueState(val current: QueueItem? = null, val waiting: List<QueueItem> = emptyList(), val results: List<ItemResult> = emptyList()) {
    val isActive: Boolean get() = current != null || waiting.isNotEmpty()
    val remaining: Int get() = waiting.size + if (current != null) 1 else 0
  }
  class SendQueue(
    scope: CoroutineScope,
    prepare: suspend (QueueItem) -> String,          // copies to cache, returns the path; throws if unreadable
    send: suspend (deviceId: String, path: String) -> Unit, // throws on failure
    cleanup: suspend (path: String) -> Unit,
  ) {
    val state: StateFlow<QueueState>
    val batches: Flow<List<ItemResult>>  // emits once each time the queue drains, the results of that run
    fun enqueue(items: List<QueueItem>)
  }
  ```
  - `results` accumulate while the queue is active and reset when the next run starts.
  - `NodeRepository` exposes `val queue: StateFlow<QueueState>` and `val batches: Flow<List<ItemResult>>`, plus `fun send(deviceId: String, deviceName: String, uris: List<Uri>)`. The display name is read up front through `OpenableColumns.DISPLAY_NAME`, falling back to the URI's last segment.

- [ ] **Step 1: Write the failing tests** in `SendQueueTest`, using `runTest` and fake lambdas that record calls:
  - `itemsRunOneAtATimeInOrder`:
    1. Enqueue A and B.
    2. While A's `send` is suspended, enqueue C.
    3. Assert `state.value.current == A` and `state.value.remaining == 3`.
    4. Resume A's send, then check that the recorded send order is `[A, B, C]` and that `prepare` for B only ran after A's `cleanup`.
  - `unreadableItemFailsAndQueueContinues`: `prepare` throws `FileNotFoundException("gone")` for B of [A, B, C]. The batch is `[ok A, B error "Could not read b.txt: gone", ok C]`, and `send` is never called for B.
  - `sendFailureIsRecordedAndCleanupStillRuns`: `send` throws `RuntimeException("refused")` for A. The batch has A with error `"refused"`, and `cleanup` ran for A's path.
  - `batchEmittedOnceWhenDrained`: two runs separated by idle produce exactly two `batches` emissions, each holding only its own results.
- [ ] **Step 2: Run** `./gradlew testDebugUnitTest --tests '*SendQueueTest*'`. Expected: compile failure (`SendQueue` unresolved).
- [ ] **Step 3: Implement `SendQueue`** with a single worker coroutine draining a `Channel`/`ArrayDeque` under a `Mutex`, and `MutableStateFlow` + `MutableSharedFlow(extraBufferCapacity = 8)` for batches. Error text: `prepare` failure → `"Could not read ${item.fileName}: ${e.message}"`; `send` failure → `e.message ?: "Send failed"`. Always call `cleanup` when `prepare` returned a path.
- [ ] **Step 4: Wire `NodeRepository`.** The queue's `prepare` is `OutgoingFiles.copyToCache(context, Uri.parse(source)).absolutePath` on `Dispatchers.IO`. Its `send` is `node.await().sendFile(deviceId, path)`, rethrowing a `ToviException`. Its `cleanup` is `File(path).parentFile?.deleteRecursively()`. Remove the old per-call coroutine. `AppViewModel.filePicked` passes the device name from `repository.devices`.
- [ ] **Step 5: Run** `./gradlew testDebugUnitTest` and then `./gradlew assembleDebug`. Expected: all tests pass and the APK builds.
- [ ] **Step 6: Commit** the files above with the message "Send queued files one at a time".

---

### Task 3: `TransferService` and notifications

**Files:**
- Create: `apps/android/app/src/main/java/app/tovi/android/service/TransferService.kt`, `apps/android/app/src/main/java/app/tovi/android/service/TransferNotifications.kt` (pure text), `apps/android/app/src/main/java/app/tovi/android/AppVisibility.kt`
- Modify: `apps/android/app/src/main/AndroidManifest.xml`, `ToviApp.kt`, `MainActivity.kt` (notification permission and hint), `data/NodeRepository.kt` (starts the service), `gradle/libs.versions.toml` + `app/build.gradle.kts` (add `androidx.lifecycle:lifecycle-process` at `androidxLifecycle`)
- Test: `apps/android/app/src/test/java/app/tovi/android/service/TransferNotificationsTest.kt`

**Interfaces:**
- Consumes: `QueueState`, `ItemResult`, `NodeRepository.queue`/`.batches`/`.live` (Task 2; `ActiveTransfer` from `LiveState.kt`).
- Produces:
  ```kotlin
  data class ProgressText(val title: String, val body: String, val percent: Int? /* null = indeterminate */)
  fun progressText(queue: QueueState, live: ActiveTransfer?): ProgressText
  data class SummaryText(val title: String, val body: String?)
  fun summaryText(batch: List<ItemResult>): SummaryText
  fun shouldPostSummary(appVisible: Boolean): Boolean
  object AppVisibility { val visible: StateFlow<Boolean> }  // fed by ProcessLifecycleOwner in ToviApp
  ```
  - `NodeRepository` starts `TransferService` when `queue.isActive` turns true.
  - `TransferService.start(context, shared: List<Uri> = emptyList(), deviceId: String? = null, deviceName: String? = null)` is the one entry point. Task 5 uses its `shared` form, which carries `ClipData` with `FLAG_GRANT_READ_URI_PERMISSION` and makes the service call `repository.send(...)`.

- [ ] **Step 1: Write the failing tests** in `TransferNotificationsTest`:
  - `preparingWhenNoLiveTransfer`: current item `photo.jpg` to `Office PC`, live `null`. Expect `ProgressText("Preparing photo.jpg…", "To Office PC", null)`.
  - `progressWithPercentAndQueue`: live 250 of 1000 bytes, remaining 3. Expect the title `"Sending photo.jpg"`, the body `"To Office PC · 2 more files"`, and percent `25`.
  - `reconnectingShown`: live `reconnecting = true`. The body starts with `"Reconnecting…"`.
  - Summary cases, exact strings from Global Constraints:
    - `summaryAllSent`: 3 ok to `Office PC`, giving `"Sent 3 files to Office PC"`;
    - `summarySingleSent`: 1 ok;
    - `summaryPartial`: 2 ok, 1 failed `b.txt` with `"refused"`, giving `"Sent 2 of 3 to Office PC · couldn't send b.txt: refused"`;
    - `summarySingleFailed`.
  - `noSummaryWhileVisible`: `shouldPostSummary(true) == false` and `shouldPostSummary(false) == true`.
- [ ] **Step 2: Run** `./gradlew testDebugUnitTest --tests '*TransferNotificationsTest*'`. Expected: compile failure.
- [ ] **Step 3: Implement the pure text functions**, then the service:
  - **Starting.** `onStartCommand` creates the channels (idempotent), then immediately calls `ServiceCompat.startForeground(this, ID_PROGRESS, notification, FOREGROUND_SERVICE_TYPE_DATA_SYNC)` with the current `progressText`.
  - **Updating.** It collects `queue` combined with `live` and updates the notification, throttled to at most 2 per second. It stops (`stopForeground(STOP_FOREGROUND_REMOVE)` + `stopSelf()`) when `!queue.isActive`.
  - **Timeout.** `onTimeout(startId, fgsType)` on API 35+ stops the service the same way.
  - **Summary.** `ToviApp` collects `batches` and posts the summary as `ID_RESULT` when `shouldPostSummary(AppVisibility.visible.value)`, with a content intent opening `MainActivity` with extra `open = "transfers"`.
  - **Opening Transfers.** `MainActivity` handles that extra by opening the Transfers tab.
- [ ] **Step 4: Manifest and permission.**
  - Add the `<service android:name=".service.TransferService" android:exported="false" android:foregroundServiceType="dataSync"/>` element and the three notification/FGS permissions.
  - In `MainActivity`, on API 33+, request `POST_NOTIFICATIONS` once before the first pick is sent. Remember that it was asked in `SharedPreferences("tovi_ui")` under key `asked_notifications`.
  - If it's denied, show the snackbar `Notifications are off, so TOVI can't show send progress` once, keyed `notification_hint_shown`.
  - A denial never blocks the send.
- [ ] **Step 5: Run** `./gradlew testDebugUnitTest` and `./gradlew assembleDebug`. Expected: pass.
- [ ] **Step 6: Commit** with the message "Keep sends running in a foreground service with progress and summary notifications".

---

### Task 4: Multicast lock and discovery on

**Files:**
- Create: `apps/android/app/src/main/java/app/tovi/android/data/MulticastLockHolder.kt`
- Modify: `data/NodeRepository.kt` (`config()` sets `discovery = true`; owns the holder), `ToviApp.kt`, `AndroidManifest.xml` (`CHANGE_WIFI_MULTICAST_STATE`)
- Test: `apps/android/app/src/test/java/app/tovi/android/data/MulticastLockHolderTest.kt`

**Interfaces:**
- Consumes: `NodeConfig.discovery` (Task 1), `AppVisibility.visible` (Task 3), `NodeRepository.queue` (Task 2).
- Produces:
  ```kotlin
  interface LockHandle { fun acquire(); fun release(); val isHeld: Boolean }
  class MulticastLockHolder(private val lock: LockHandle) {
    fun update(appVisible: Boolean, sending: Boolean)  // held iff either is true; idempotent
  }
  fun wifiMulticastLock(context: Context): LockHandle  // WifiManager.createMulticastLock("tovi").apply { setReferenceCounted(false) }
  ```

- [ ] **Step 1: Write the failing tests** with a fake `LockHandle` that counts calls:
  - `heldWhileVisible`;
  - `heldWhileSending`;
  - `lockHeldUntilQueueEmptiesAfterBackgrounding`: the sequence (visible, not sending) → (visible, sending) → (hidden, sending) → (hidden, not sending) gives exactly 1 `acquire` and 1 `release`, and the lock is still held after the third update;
  - `repeatedUpdatesDoNotReacquire`.
- [ ] **Step 2: Run** `./gradlew testDebugUnitTest --tests '*MulticastLockHolderTest*'`. Expected: compile failure.
- [ ] **Step 3: Implement** the holder. In `NodeRepository.start`, combine `AppVisibility.visible` with `queue.map { it.isActive }` and call `holder.update`. Set `discovery = true` in `config()`. Add the manifest permission.
- [ ] **Step 4: Run** `./gradlew testDebugUnitTest` and `./gradlew assembleDebug`. Expected: pass.
- [ ] **Step 5: Commit** with the message "Turn on discovery on Android and hold the multicast lock only when needed".

---

### Task 5: Share → TOVI

**Files:**
- Create: `apps/android/app/src/main/java/app/tovi/android/share/ShareActivity.kt`, `apps/android/app/src/main/java/app/tovi/android/share/SharedStreams.kt`
- Modify: `AndroidManifest.xml`
- Test: `apps/android/app/src/test/java/app/tovi/android/share/SharedStreamsTest.kt`

**Interfaces:**
- Consumes: `NodeRepository.devices`, `TransferService.start(context, shared, deviceId, deviceName)` (Task 3).
- Produces: `fun <T> sharedStreams(action: String?, single: T?, multiple: List<T>?): List<T>` (pure).

- [ ] **Step 1: Write the failing tests** for `sharedStreams`:
  - `ACTION_SEND` with a single item gives `[single]`;
  - `ACTION_SEND_MULTIPLE` with `[a, b, c]` gives the same list in the same order;
  - `ACTION_SEND` with `null` (a text share) gives `[]`;
  - any other action gives `[]`;
  - `null`s inside `multiple` are dropped.
  Use the literal action strings `"android.intent.action.SEND"` and `"android.intent.action.SEND_MULTIPLE"`.
- [ ] **Step 2: Run** `./gradlew testDebugUnitTest --tests '*SharedStreamsTest*'`. Expected: compile failure.
- [ ] **Step 3: Implement.**
  - **The picker.** `ShareActivity` is a `ComponentActivity` with a Compose list titled `Send N files to…` (`Send <file> to…` for one). It shows the paired devices from `repository.devices`.
  - **Choosing a device.** Request notifications as in Task 3 Step 4, then `TransferService.start(this, uris, device.id, device.name)`, then `finish()`.
  - **No paired devices.** Show `Pair a device in TOVI first` with an `Open TOVI` button that launches `MainActivity`.
  - **No streams.** Finish immediately.
  - **Manifest.** Add `android:exported="true"`, `android:excludeFromRecents="true"`, `android:theme="@style/Theme.TOVI"`, and intent filters for `SEND` and `SEND_MULTIPLE` with `mimeType="*/*"`.
- [ ] **Step 4: Run** `./gradlew testDebugUnitTest` and `./gradlew assembleDebug`. Expected: pass.
- [ ] **Step 5: Commit** with the message "Add Share → TOVI".

---

### Task 6: Docs, full gates, phone checklist

**Files:**
- Modify: `docs/TASKS.md` §11, `apps/android/README.md`

- [ ] **Step 1: Update `TASKS.md` §11.**
  - Tick the share-sheet, foreground-service and permissions items.
  - Reword the NSD item to "Discovery on Android (Rust mDNS + MulticastLock; NsdManager fallback if it fails on devices)". Leave it unticked until the phone check passes.
  - Reword the permissions item so it no longer mentions `NEARBY_WIFI_DEVICES`. Note it's only needed for Wi‑Fi Direct/Aware later.
- [ ] **Step 2: Update `apps/android/README.md`.** Cover the service, sharing, discovery and the multicast lock. Add a "Checking on a phone" list with the spec's five on-device checks, verbatim from the spec's Testing section.
- [ ] **Step 3: Run all the Rust and Android gates.** Expected: pass.
- [ ] **Step 4: Commit** with the message "Document the Android background, share and discovery work".
