package app.tovi.android.service

import app.tovi.android.data.ActiveTransfer
import app.tovi.android.data.ItemResult
import app.tovi.android.data.QueueState

/** What the ongoing progress notification says; [percent] is null while indeterminate */
data class ProgressText(val title: String, val body: String, val percent: Int?)

fun progressText(queue: QueueState, live: ActiveTransfer?): ProgressText {
  val item = queue.current ?: queue.waiting.firstOrNull() ?: return ProgressText("Preparing…", "", null)
  val more = queue.remaining - 1
  val tail = if (more > 0) " · $more more ${if (more == 1) "file" else "files"}" else ""
  val to = "To ${item.deviceName}"
  if (live == null) return ProgressText("Preparing ${item.fileName}…", to + tail, null)
  val body = (if (live.reconnecting) "Reconnecting… · " else "") + to + tail
  val percent = if (live.total > 0) (live.fraction * 100).toInt() else null
  return ProgressText("Sending ${live.fileName}", body, percent)
}

/** The notification posted when a run of the queue ends */
data class SummaryText(val title: String, val body: String?)

fun summaryText(batch: List<ItemResult>): SummaryText {
  if (batch.isEmpty()) return SummaryText("Nothing was sent", null)
  val device = batch.first().item.deviceName
  val firstFailed = batch.firstOrNull { it.error != null }
  val title =
    when {
      firstFailed == null ->
        if (batch.size == 1) "Sent ${batch[0].item.fileName} to $device" else "Sent ${batch.size} files to $device"
      batch.size == 1 -> "Couldn't send ${firstFailed.item.fileName}: ${firstFailed.error}"
      else ->
        "Sent ${batch.count { it.error == null }} of ${batch.size} to $device · " +
          "couldn't send ${firstFailed.item.fileName}: ${firstFailed.error}"
    }
  return SummaryText(title, null)
}

/** While the app is on screen the snackbar already says it */
fun shouldPostSummary(appVisible: Boolean): Boolean = !appVisible
