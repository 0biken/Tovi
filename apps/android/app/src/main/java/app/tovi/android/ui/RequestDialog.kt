package app.tovi.android.ui

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.window.DialogProperties
import app.tovi.android.data.PendingRequest

/** Asks the user to allow a pairing or an incoming file; unanswered requests expire after 60 s */
@Composable
fun RequestDialog(request: PendingRequest, onRespond: (requestId: ULong, allow: Boolean) -> Unit) {
  val (title, body, allowLabel) =
    when (request) {
      is PendingRequest.Pairing ->
        Triple(
          "Pair with ${request.deviceName}?",
          "${platformLabel(request.platform)} device ${request.shortId} wants to pair. Only allow it if you just showed it this phone's code.",
          "Allow",
        )
      is PendingRequest.IncomingFile ->
        Triple("Receive a file?", "${request.deviceName} wants to send ${request.fileName} (${formatBytes(request.fileSize)}).", "Accept")
    }
  AlertDialog(
    // Answering is required: dismissing by tapping outside would leave it pending
    onDismissRequest = {},
    properties = DialogProperties(dismissOnClickOutside = false),
    title = { Text(title) },
    text = { Text(body) },
    confirmButton = { TextButton(onClick = { onRespond(request.requestId, true) }) { Text(allowLabel) } },
    dismissButton = { TextButton(onClick = { onRespond(request.requestId, false) }) { Text("Decline") } },
  )
}
