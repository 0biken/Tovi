package app.tovi.android.service

import app.tovi.android.data.ActiveTransfer
import app.tovi.android.data.ItemResult
import app.tovi.android.data.QueueItem
import app.tovi.android.data.QueueState
import app.tovi.core.Direction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TransferNotificationsTest {
  private fun item(name: String) = QueueItem("d1", "Office PC", "content://x/$name", name)

  private fun live(done: Long = 250, reconnecting: Boolean = false) =
    ActiveTransfer("t1", Direction.SENT, "Office PC", "photo.jpg", total = 1000, done = done, reconnecting = reconnecting)

  private fun ok(name: String) = ItemResult(item(name), null)

  private fun failed(name: String, error: String) = ItemResult(item(name), error)

  @Test
  fun preparingWhenNoLiveTransfer() {
    val queue = QueueState(current = item("photo.jpg"))
    assertEquals(ProgressText("Preparing photo.jpg…", "To Office PC", null), progressText(queue, null))
  }

  @Test
  fun progressWithPercentAndQueue() {
    val queue = QueueState(current = item("photo.jpg"), waiting = listOf(item("a"), item("b")))
    assertEquals(ProgressText("Sending photo.jpg", "To Office PC · 2 more files", 25), progressText(queue, live()))
  }

  @Test
  fun reconnectingShown() {
    val queue = QueueState(current = item("photo.jpg"))
    val text = progressText(queue, live(reconnecting = true))
    assertTrue(text.body.startsWith("Reconnecting…"))
  }

  @Test
  fun summaryAllSent() {
    val batch = listOf(ok("a"), ok("b"), ok("c"))
    assertEquals(SummaryText("Sent 3 files to Office PC", null), summaryText(batch))
  }

  @Test
  fun summarySingleSent() {
    assertEquals(SummaryText("Sent a.txt to Office PC", null), summaryText(listOf(ok("a.txt"))))
  }

  @Test
  fun summaryPartial() {
    val batch = listOf(ok("a.txt"), failed("b.txt", "refused"), ok("c.txt"))
    assertEquals(
      SummaryText("Sent 2 of 3 to Office PC · couldn't send b.txt: refused", null),
      summaryText(batch),
    )
  }

  @Test
  fun summarySingleFailed() {
    assertEquals(SummaryText("Couldn't send b.txt: refused", null), summaryText(listOf(failed("b.txt", "refused"))))
  }

  @Test
  fun noSummaryWhileVisible() {
    assertFalse(shouldPostSummary(true))
    assertTrue(shouldPostSummary(false))
  }

  @Test
  fun summaryWaitsLongerThanTheLifecycleStopDelay() {
    // ProcessLifecycleOwner reports a stop ~700 ms after the last activity stops
    assertTrue(SUMMARY_SETTLE_MS >= 1000L)
  }
}
