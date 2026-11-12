package app.tovi.android.data

import java.io.FileNotFoundException
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
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
  fun prepareFailureIsReportedToTheCallback() = runTest {
    val a = item("a.txt")
    val reported = mutableListOf<ItemResult>()
    val queue =
      SendQueue(
        scope = backgroundScope,
        prepare = { throw FileNotFoundException("gone") },
        send = { _, _ -> },
        cleanup = {},
        onPrepareFailed = { reported += it },
      )
    queue.enqueue(listOf(a))
    runCurrent()

    assertEquals(listOf(ItemResult(a, "Could not read a.txt: gone")), reported)
  }

  @Test
  fun throwingCleanupDoesNotStopTheQueue() = runTest {
    val items = listOf(item("a.txt"), item("b.txt"))
    val queue =
      SendQueue(
        scope = backgroundScope,
        prepare = { it.fileName },
        send = { _, _ -> },
        cleanup = { throw java.io.IOException("busy") },
      )
    val batches = mutableListOf<List<ItemResult>>()
    backgroundScope.launch { queue.batches.collect { batches += it } }
    runCurrent()

    queue.enqueue(items)
    runCurrent()

    assertEquals(listOf(ItemResult(items[0], null), ItemResult(items[1], null)), batches.single())
    assertNull(queue.state.value.current)
  }

  @Test
  fun cancelledWorkerLeavesTheQueueInactive() = runTest {
    val a = item("a.txt")
    val scope = CoroutineScope(SupervisorJob())
    val started = CompletableDeferred<Unit>()
    val queue =
      SendQueue(
        scope = scope,
        prepare = { it.fileName },
        send = { _, _ ->
          started.complete(Unit)
          CompletableDeferred<Unit>().await()
        },
        cleanup = {},
      )
    queue.enqueue(listOf(a))
    started.await()
    scope.cancel()
    scope.coroutineContext[kotlinx.coroutines.Job]!!.join()

    assertEquals(false, queue.state.value.isActive)
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

  @Test
  fun enqueueFromInsideTheLastItemsSendIsStillProcessed() = runTest {
    val a = item("a.txt")
    val b = item("b.txt")
    val sent = mutableListOf<String>()
    lateinit var queue: SendQueue
    queue =
      SendQueue(
        scope = backgroundScope,
        prepare = { it.fileName },
        send = { _, path ->
          sent += path
          if (path == "a.txt") queue.enqueue(listOf(b))
        },
        cleanup = {},
      )

    queue.enqueue(listOf(a))
    runCurrent()

    assertEquals(listOf("a.txt", "b.txt"), sent)
    assertEquals(false, queue.state.value.isActive)
  }

  /** An enqueue racing the moment the worker finishes must start a new worker or be picked up, never be stranded */
  @Test
  fun enqueueRacingTheDrainIsNeverStranded() {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    val sent = AtomicInteger()
    val queue =
      SendQueue(scope = scope, prepare = { it.fileName }, send = { _, _ -> sent.incrementAndGet() }, cleanup = {})
    try {
      runBlocking {
        repeat(5_000) { i ->
          queue.enqueue(listOf(item("a$i")))
          repeat(i % 400) { Thread.onSpinWait() }
          queue.enqueue(listOf(item("b$i")))
          val deadline = System.nanoTime() + 5_000_000_000L
          while (sent.get() < 2 * (i + 1) || queue.state.value.isActive) {
            check(System.nanoTime() < deadline) { "queue stranded an item at run $i: ${queue.state.value}" }
            Thread.yield()
          }
        }
      }
    } finally {
      scope.cancel()
    }
  }
}
