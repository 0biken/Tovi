package app.tovi.android.data

import java.io.FileNotFoundException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class SendQueueTest {
  private fun item(name: String) = QueueItem("dev", "Laptop", "content://x/$name", name)

  @Test
  fun itemsRunOneAtATimeInOrder() = runTest {
    val a = item("a.txt")
    val b = item("b.txt")
    val c = item("c.txt")
    val events = mutableListOf<String>()
    val gate = CompletableDeferred<Unit>()
    val queue =
      SendQueue(
        scope = backgroundScope,
        prepare = { events += "prepare ${it.fileName}"; it.fileName },
        send = { _, path ->
          events += "send $path"
          if (path == "a.txt") gate.await()
        },
        cleanup = { events += "cleanup $it" },
      )

    queue.enqueue(listOf(a, b))
    runCurrent()
    queue.enqueue(listOf(c))
    runCurrent()
    assertEquals(a, queue.state.value.current)
    assertEquals(3, queue.state.value.remaining)

    gate.complete(Unit)
    runCurrent()

    assertEquals(
      listOf(
        "prepare a.txt", "send a.txt", "cleanup a.txt",
        "prepare b.txt", "send b.txt", "cleanup b.txt",
        "prepare c.txt", "send c.txt", "cleanup c.txt",
      ),
      events,
    )
    assertEquals(false, queue.state.value.isActive)
  }

  @Test
  fun unreadableItemFailsAndQueueContinues() = runTest {
    val items = listOf(item("a.txt"), item("b.txt"), item("c.txt"))
    val sent = mutableListOf<String>()
    val queue =
      SendQueue(
        scope = backgroundScope,
        prepare = { if (it.fileName == "b.txt") throw FileNotFoundException("gone") else it.fileName },
        send = { _, path -> sent += path },
        cleanup = {},
      )
    val batches = mutableListOf<List<ItemResult>>()
    backgroundScope.launch { queue.batches.collect { batches += it } }
    runCurrent()

    queue.enqueue(items)
    runCurrent()

    assertEquals(
      listOf(
        ItemResult(items[0], null),
        ItemResult(items[1], "Could not read b.txt: gone"),
        ItemResult(items[2], null),
      ),
      batches.single(),
    )
    assertEquals(listOf("a.txt", "c.txt"), sent)
  }

  @Test
  fun sendFailureIsRecordedAndCleanupStillRuns() = runTest {
    val a = item("a.txt")
    val cleaned = mutableListOf<String>()
    val queue =
      SendQueue(
        scope = backgroundScope,
        prepare = { "/cache/${it.fileName}" },
        send = { _, _ -> throw RuntimeException("refused") },
        cleanup = { cleaned += it },
      )
    val batches = mutableListOf<List<ItemResult>>()
    backgroundScope.launch { queue.batches.collect { batches += it } }
    runCurrent()

    queue.enqueue(listOf(a))
    runCurrent()

    assertEquals(listOf(ItemResult(a, "refused")), batches.single())
    assertEquals(listOf("/cache/a.txt"), cleaned)
    assertNull(queue.state.value.current)
  }

  @Test
  fun batchEmittedOnceWhenDrained() = runTest {
    val a = item("a.txt")
    val b = item("b.txt")
    val queue =
      SendQueue(scope = backgroundScope, prepare = { it.fileName }, send = { _, _ -> }, cleanup = {})
    val batches = mutableListOf<List<ItemResult>>()
    backgroundScope.launch { queue.batches.collect { batches += it } }
    runCurrent()

    queue.enqueue(listOf(a))
    runCurrent()
    queue.enqueue(listOf(b))
    runCurrent()

    assertEquals(listOf(listOf(ItemResult(a, null)), listOf(ItemResult(b, null))), batches)
  }
}
