package app.tovi.android.data

import app.tovi.core.Direction
import app.tovi.core.NodeEvent
import app.tovi.core.TransferOutcome
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class LiveStateTest {
  private val started =
    NodeEvent.TransferStarted(
      transferId = "t1",
      direction = Direction.RECEIVED,
      deviceId = "d".repeat(64),
      deviceName = "Office PC",
      fileName = "video.mp4",
      fileSize = 1000u,
      resuming = false,
    )

  private fun finished(outcome: TransferOutcome, direction: Direction = Direction.RECEIVED) =
    NodeEvent.TransferFinished("t1", direction, "d".repeat(64), "Office PC", "video.mp4", outcome)

  @Test
  fun transferLifecycle() {
    var state = LiveState().after(started)
    assertEquals(ActiveTransfer("t1", Direction.RECEIVED, "Office PC", "video.mp4", total = 1000), state.transfers["t1"])

    state = state.after(NodeEvent.TransferProgress("t1", Direction.RECEIVED, 250u, 1000u, 50u))
    assertEquals(0.25f, state.transfers.getValue("t1").fraction)
    assertEquals(50L, state.transfers.getValue("t1").bytesPerSec)

    state = state.after(NodeEvent.TransferReconnecting("t1", "timed out"))
    assertTrue(state.transfers.getValue("t1").reconnecting)

    // Progress again means the connection is back
    state = state.after(NodeEvent.TransferProgress("t1", Direction.RECEIVED, 500u, 1000u, 40u))
    assertEquals(false, state.transfers.getValue("t1").reconnecting)

    state = state.after(finished(TransferOutcome.Completed("/x/video.mp4", 1000u, 0u)))
    assertTrue(state.transfers.isEmpty())
  }

  @Test
  fun progressForAnUnknownTransferIsIgnored() {
    val state = LiveState().after(NodeEvent.TransferProgress("nope", Direction.SENT, 1u, 2u, 3u))
    assertEquals(LiveState(), state)
  }

  @Test
  fun requestsQueueUntilAnsweredOrExpired() {
    var state =
      LiveState()
        .after(NodeEvent.PairingRequested(1u, "abcdef0123".padEnd(64, '0'), "MacBook", "macos"))
        .after(NodeEvent.IncomingOffer(2u, "t2", "d".repeat(64), "Office PC", "notes.txt", 42u))
    assertEquals(
      listOf(
        PendingRequest.Pairing(1u, "MacBook", "macos", "abcdef01"),
        PendingRequest.IncomingFile(2u, "Office PC", "notes.txt", 42),
      ),
      state.requests,
    )

    state = state.after(NodeEvent.RequestExpired(1u))
    assertEquals(listOf(2uL), state.requests.map { it.requestId })
    assertTrue(state.withoutRequest(2u).requests.isEmpty())
  }

  @Test
  fun reactions() {
    assertEquals(Reaction(reloadDevices = true), reactionTo(NodeEvent.DevicesChanged))
    assertEquals(Reaction(reloadDevices = true, reloadHistory = true), reactionTo(NodeEvent.Lagged(3u)))
    assertEquals(Reaction(reloadHistory = true), reactionTo(started))
    assertEquals("Paired with MacBook", reactionTo(NodeEvent.Paired("x", "MacBook", "macos")).notice)

    assertEquals("Received video.mp4", reactionTo(finished(TransferOutcome.Completed("/x", 1u, 0u))).notice)
    assertEquals("video.mp4 failed: declined", reactionTo(finished(TransferOutcome.Failed("declined"))).notice)
    // The sendFile call reports its own failure
    val sendFailed = reactionTo(finished(TransferOutcome.Failed("declined"), Direction.SENT))
    assertNull(sendFailed.notice)
    assertTrue(sendFailed.reloadHistory)
  }
}
