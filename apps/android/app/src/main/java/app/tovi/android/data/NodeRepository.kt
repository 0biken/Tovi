package app.tovi.android.data

import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.OpenableColumns
import android.provider.Settings
import android.util.Log
import app.tovi.android.service.TransferService
import app.tovi.core.Device
import app.tovi.core.EventListener
import app.tovi.core.LocalId
import app.tovi.core.NodeConfig
import app.tovi.core.NodeEvent
import app.tovi.core.ToviException
import app.tovi.core.ToviNode
import app.tovi.core.TransferRecord
import java.io.File
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The one [ToviNode] in this process, and what the UI (and later the
 * foreground service) observe of it. Events arrive on a Rust thread, are
 * queued, and are applied in order on [scope].
 */
class NodeRepository(private val context: Context, private val scope: CoroutineScope) {
  sealed interface Status {
    data object Starting : Status

    data class Running(val local: LocalId, val port: Int) : Status

    data class Failed(val message: String) : Status
  }

  private val node = CompletableDeferred<ToviNode>()
  private val events = Channel<NodeEvent>(Channel.UNLIMITED)

  private val _status = MutableStateFlow<Status>(Status.Starting)
  val status: StateFlow<Status> = _status.asStateFlow()

  private val _devices = MutableStateFlow<List<Device>>(emptyList())
  val devices: StateFlow<List<Device>> = _devices.asStateFlow()

  private val _history = MutableStateFlow<List<TransferRecord>>(emptyList())
  val history: StateFlow<List<TransferRecord>> = _history.asStateFlow()

  private val _live = MutableStateFlow(LiveState())
  val live: StateFlow<LiveState> = _live.asStateFlow()

  private val _autoAccept = MutableStateFlow(true)
  val autoAccept: StateFlow<Boolean> = _autoAccept.asStateFlow()

  private val _receiveDir = MutableStateFlow("")
  val receiveDir: StateFlow<String> = _receiveDir.asStateFlow()

  // Buffered, so a message raised before the UI subscribes (say, while the
  // activity is being recreated) waits for it instead of being dropped
  private val _notices = Channel<String>(capacity = 16, onBufferOverflow = BufferOverflow.DROP_OLDEST)
  /** Short messages for the user (snackbars); meant for a single collector */
  val notices: Flow<String> = _notices.receiveAsFlow()

  /** Called once, from [app.tovi.android.ToviApp.onCreate] */
  fun start() {
    scope.launch {
      for (event in events) handle(event)
    }
    // The service keeps the process alive for as long as the queue has work
    scope.launch {
      sendQueue.state
        .map { it.isActive }
        .distinctUntilChanged()
        .filter { it }
        .collect { TransferService.start(context) }
    }
    scope.launch(Dispatchers.IO) {
      try {
        val started = ToviNode.start(config(), Listener(events))
        node.complete(started)
        _status.value = Status.Running(started.localId(), started.port().toInt())
        _autoAccept.value = started.autoAccept()
        _receiveDir.value = started.receiveDir()
        reloadDevices()
        reloadHistory()
      } catch (e: ToviException) {
        Log.e(TAG, "could not start the node", e)
        _status.value = Status.Failed(e.message ?: "TOVI could not start")
      }
    }
  }

  // ------------------------------------------------------------------ actions

  /** Pair with the device whose `tovi://pair/...` code was scanned or pasted. A failure is the caller's to show. */
  suspend fun pair(code: String): Result<Device> =
    call(notify = false) { it.pair(code) }.onSuccess { reloadDevices() }

  /** Answer a [PendingRequest] */
  fun respond(requestId: ULong, allow: Boolean) {
    _live.update { it.withoutRequest(requestId) }
    scope.launch {
      if (!node.await().respond(requestId, allow)) _notices.trySend("That request already expired")
    }
  }

  private val sendQueue =
    SendQueue(
      scope = scope,
      prepare = { item ->
        withContext(Dispatchers.IO) { OutgoingFiles.copyToCache(context, Uri.parse(item.source)).absolutePath }
      },
      send = { deviceId, path ->
        // Success and failure are also reported by TransferFinished events
        call { it.sendFile(deviceId, path) }.getOrThrow()
      },
      cleanup = { path -> withContext(Dispatchers.IO + NonCancellable) { File(path).parentFile?.deleteRecursively() } },
      // Nothing reaches the core for such an item, so history never shows it: say so now
      onPrepareFailed = { result -> result.error?.let { _notices.trySend(it) } },
    )

  /** What the send queue is doing right now */
  val queue: StateFlow<QueueState> = sendQueue.state

  /** The results of each run of the send queue, once it drains */
  val batches: Flow<List<ItemResult>> = sendQueue.batches

  /**
   * Queue documents picked or shared by the user. The core reads files by
   * path, so each is copied into the app's cache just before it is sent and
   * removed afterwards.
   */
  fun send(deviceId: String, deviceName: String, uris: List<Uri>) {
    scope.launch { sendNow(deviceId, deviceName, uris) }
  }

  /** [send], for a caller that must know the files are queued before it carries on */
  suspend fun sendNow(deviceId: String, deviceName: String, uris: List<Uri>) {
    Log.i(TAG, "queueing ${uris.size} file(s) for ${deviceId.take(8)}")
    val items =
      withContext(Dispatchers.IO) { uris.map { QueueItem(deviceId, deviceName, it.toString(), displayName(it)) } }
    sendQueue.enqueue(items)
  }

  private fun displayName(uri: Uri): String {
    val name =
      runCatching {
          context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
          }
        }
        .getOrNull()
    return OutgoingFiles.safeName(name ?: uri.lastPathSegment ?: "file")
  }

  fun forget(deviceId: String) {
    scope.launch { call { it.forget(deviceId) } }
  }

  fun setAutoAccept(enabled: Boolean) {
    scope.launch { call { it.setAutoAccept(enabled) }.onSuccess { _autoAccept.value = enabled } }
  }

  // ------------------------------------------------------------------ events

  private suspend fun handle(event: NodeEvent) {
    _live.update { it.after(event) }
    val reaction = reactionTo(event)
    if (reaction.reloadDevices) reloadDevices()
    if (reaction.reloadHistory) reloadHistory()
    reaction.notice?.let { _notices.trySend(it) }
  }

  private suspend fun reloadDevices() {
    call { it.devices() }.onSuccess { _devices.value = it }
  }

  private suspend fun reloadHistory() {
    call { it.history(HISTORY_LIMIT) }.onSuccess { _history.value = it }
  }

  /**
   * Run [block] against the node off the main thread. A [ToviException] is
   * logged, shown to the user if [notify], and returned as a failure.
   */
  private suspend fun <T> call(notify: Boolean = true, block: suspend (ToviNode) -> T): Result<T> {
    val node = node.await()
    return try {
      Result.success(withContext(Dispatchers.IO) { block(node) })
    } catch (e: ToviException) {
      Log.w(TAG, "node call failed", e)
      if (notify) _notices.trySend(e.message ?: "Something went wrong")
      Result.failure(e)
    }
  }

  private fun config(): NodeConfig {
    val dataDir = File(context.filesDir, "tovi")
    // App-specific external storage: a real path, no storage permission
    val downloads = context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: context.filesDir
    return NodeConfig(
      dataDir = dataDir.absolutePath,
      deviceName = deviceName(),
      defaultReceiveDir = File(downloads, "TOVI").absolutePath,
      listen = null,
      discovery = true,
    )
  }

  /** The name the user gave this phone, as other devices will see it */
  private fun deviceName(): String =
    Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME)?.takeIf { it.isNotBlank() }
      ?: listOf(Build.MANUFACTURER.replaceFirstChar { it.uppercase() }, Build.MODEL).distinct().joinToString(" ")

  /** Runs on a Rust thread: hand the event over and return straight away */
  private class Listener(private val events: Channel<NodeEvent>) : EventListener {
    override fun onEvent(event: NodeEvent) {
      events.trySend(event)
    }
  }

  private companion object {
    const val TAG = "Tovi"
    const val HISTORY_LIMIT = 200u
  }
}
