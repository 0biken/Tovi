package app.tovi.android.data

import app.tovi.core.Direction
import app.tovi.core.NodeEvent
import app.tovi.core.TransferOutcome

/** A transfer in progress, built up from `TransferStarted` / `Progress` events */
data class ActiveTransfer(
  val id: String,
  val direction: Direction,
  val deviceName: String,
  val fileName: String,
  val total: Long,
  val done: Long = 0,
  val bytesPerSec: Long = 0,
  /** The connection dropped and the sender is reconnecting */
  val reconnecting: Boolean = false,
) {
  val fraction: Float
    get() = if (total > 0) (done.toFloat() / total).coerceIn(0f, 1f) else 0f
}

/** Something the core is waiting for the user to answer with `respond` */
sealed interface PendingRequest {
  val requestId: ULong

  data class Pairing(
    override val requestId: ULong,
    val deviceName: String,
    val platform: String,
    val shortId: String,
  ) : PendingRequest

  data class IncomingFile(
    override val requestId: ULong,
    val deviceName: String,
    val fileName: String,
    val fileSize: Long,
  ) : PendingRequest
}

/** What the event stream says right now; devices and history are reloaded from the node */
data class LiveState(
  val transfers: Map<String, ActiveTransfer> = emptyMap(),
  /** Oldest first */
  val requests: List<PendingRequest> = emptyList(),
) {
  fun withoutRequest(requestId: ULong) = copy(requests = requests.filterNot { it.requestId == requestId })
}

/** Follow-up work an event calls for, besides updating [LiveState] */
data class Reaction(
  val reloadDevices: Boolean = false,
  val reloadHistory: Boolean = false,
  /** A short message for the user */
  val notice: String? = null,
)

fun LiveState.after(event: NodeEvent): LiveState =
  when (event) {
    is NodeEvent.PairingRequested ->
      copy(requests = requests + PendingRequest.Pairing(event.requestId, event.deviceName, event.platform, event.deviceId.take(8)))
    is NodeEvent.IncomingOffer ->
      copy(requests = requests + PendingRequest.IncomingFile(event.requestId, event.deviceName, event.fileName, event.fileSize.toLong()))
    is NodeEvent.RequestExpired -> withoutRequest(event.requestId)
    is NodeEvent.TransferStarted ->
      copy(
        transfers =
          transfers +
            (event.transferId to
              ActiveTransfer(event.transferId, event.direction, event.deviceName, event.fileName, event.fileSize.toLong()))
      )
    is NodeEvent.TransferProgress -> {
      val transfer = transfers[event.transferId]
      if (transfer == null) {
        this
      } else {
        val updated =
          transfer.copy(
            done = event.done.toLong(),
            total = event.total.toLong(),
            bytesPerSec = event.bytesPerSec.toLong(),
            reconnecting = false,
          )
        copy(transfers = transfers + (event.transferId to updated))
      }
    }
    is NodeEvent.TransferReconnecting -> {
      val transfer = transfers[event.transferId]
      if (transfer == null) this else copy(transfers = transfers + (event.transferId to transfer.copy(reconnecting = true)))
    }
    is NodeEvent.TransferFinished -> copy(transfers = transfers - event.transferId)
    is NodeEvent.Paired,
    is NodeEvent.PairingFailed,
    is NodeEvent.DevicesChanged,
    is NodeEvent.Lagged -> this
  }

fun reactionTo(event: NodeEvent): Reaction =
  when (event) {
    is NodeEvent.Paired -> Reaction(reloadDevices = true, notice = "Paired with ${event.deviceName}")
    is NodeEvent.PairingFailed -> Reaction(notice = "Pairing with ${event.shortId} failed: ${event.error}")
    is NodeEvent.DevicesChanged -> Reaction(reloadDevices = true)
    is NodeEvent.TransferStarted -> Reaction(reloadHistory = true)
    is NodeEvent.TransferFinished ->
      Reaction(
        reloadHistory = true,
        notice =
          when (val outcome = event.outcome) {
            is TransferOutcome.Completed ->
              if (event.direction == Direction.RECEIVED) "Received ${event.fileName}" else "Sent ${event.fileName}"
            // A failed send is reported by the sendFile call that started it
            is TransferOutcome.Failed ->
              if (event.direction == Direction.RECEIVED) "${event.fileName} failed: ${outcome.error}" else null
            is TransferOutcome.Interrupted ->
              if (event.direction == Direction.RECEIVED) "${event.fileName} was interrupted: ${outcome.error}" else null
          },
      )
    is NodeEvent.Lagged -> Reaction(reloadDevices = true, reloadHistory = true)
    is NodeEvent.PairingRequested,
    is NodeEvent.IncomingOffer,
    is NodeEvent.RequestExpired,
    is NodeEvent.TransferProgress,
    is NodeEvent.TransferReconnecting -> Reaction()
  }
