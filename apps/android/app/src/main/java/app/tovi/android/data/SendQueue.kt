package app.tovi.android.data

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** One file waiting to be sent; [source] is a content URI string */
data class QueueItem(val deviceId: String, val deviceName: String, val source: String, val fileName: String)

/** How one item ended: [error] is null when it was sent */
data class ItemResult(val item: QueueItem, val error: String?)

data class QueueState(
  val current: QueueItem? = null,
  val waiting: List<QueueItem> = emptyList(),
  val results: List<ItemResult> = emptyList(),
) {
  val isActive: Boolean get() = current != null || waiting.isNotEmpty()
  val remaining: Int get() = waiting.size + if (current != null) 1 else 0
}

/**
 * Sends files one at a time, in the order they were enqueued. A failed item
 * is recorded and the rest carry on. Free of Android types so it runs in JVM tests.
 */
class SendQueue(
  private val scope: CoroutineScope,
  private val prepare: suspend (QueueItem) -> String,
  private val send: suspend (deviceId: String, path: String) -> Unit,
  private val cleanup: suspend (path: String) -> Unit,
) {
  private val lock = Any()
  private val _state = MutableStateFlow(QueueState())
  val state: StateFlow<QueueState> = _state.asStateFlow()

  private val _batches = MutableSharedFlow<List<ItemResult>>(extraBufferCapacity = 8)
  /** Emits once each time the queue drains: the results of that run */
  val batches: Flow<List<ItemResult>> = _batches.asSharedFlow()

  fun enqueue(items: List<QueueItem>) {
    if (items.isEmpty()) return
    val startWorker =
      synchronized(lock) {
        val before = _state.value
        // A new run starts from nothing: last run's results are dropped
        _state.value =
          if (before.isActive) before.copy(waiting = before.waiting + items)
          else QueueState(waiting = items)
        !before.isActive
      }
    if (startWorker) scope.launch { drain() }
  }

  private suspend fun drain() {
    while (true) {
      val item =
        synchronized(lock) {
          val s = _state.value
          val next = s.waiting.firstOrNull() ?: return@synchronized null
          _state.value = s.copy(current = next, waiting = s.waiting.drop(1))
          next
        } ?: break
      val result = run(item)
      synchronized(lock) { _state.value = _state.value.let { it.copy(results = it.results + result) } }
    }
    val finished =
      synchronized(lock) {
        val s = _state.value
        _state.value = s.copy(current = null)
        s.results
      }
    _batches.emit(finished)
  }

  private suspend fun run(item: QueueItem): ItemResult {
    val path =
      try {
        prepare(item)
      } catch (e: CancellationException) {
        throw e
      } catch (e: Exception) {
        return ItemResult(item, "Could not read ${item.fileName}: ${e.message}")
      }
    return try {
      send(item.deviceId, path)
      ItemResult(item, null)
    } catch (e: CancellationException) {
      throw e
    } catch (e: Exception) {
      ItemResult(item, e.message ?: "Send failed")
    } finally {
      cleanup(path)
    }
  }
}
