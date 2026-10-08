package app.tovi.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Card
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import app.tovi.android.data.ActiveTransfer
import app.tovi.android.theme.TOVITheme
import app.tovi.core.Direction
import app.tovi.core.TransferRecord
import app.tovi.core.TransferStatus

@Composable
fun TransfersScreen(active: List<ActiveTransfer>, history: List<TransferRecord>, modifier: Modifier = Modifier) {
  // History rows for transfers still running are shown as active cards instead
  val activeIds = active.map { it.id }.toSet()
  val finished = history.filterNot { it.id in activeIds }

  LazyColumn(modifier.fillMaxSize()) {
    if (active.isEmpty() && finished.isEmpty()) {
      item {
        Text(
          "No transfers yet.",
          style = MaterialTheme.typography.bodyMedium,
          color = MaterialTheme.colorScheme.onSurfaceVariant,
          textAlign = TextAlign.Center,
          modifier = Modifier.fillMaxWidth().padding(32.dp),
        )
      }
    }
    items(active, key = { "active-${it.id}" }) { ActiveTransferCard(it) }
    if (finished.isNotEmpty()) {
      item { SectionTitle("History") }
    }
    items(finished, key = { it.id }) { record ->
      HistoryRow(record)
      HorizontalDivider()
    }
  }
}

@Composable
private fun SectionTitle(text: String) {
  Text(
    text,
    style = MaterialTheme.typography.titleSmall,
    color = MaterialTheme.colorScheme.primary,
    modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
  )
}

@Composable
private fun ActiveTransferCard(transfer: ActiveTransfer) {
  Card(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
      Text(transfer.fileName, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
      Text(
        (if (transfer.direction == Direction.SENT) "To " else "From ") + transfer.deviceName,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
      )
      LinearProgressIndicator(progress = { transfer.fraction }, modifier = Modifier.fillMaxWidth())
      Text(
        if (transfer.reconnecting) {
          "Connection lost, reconnecting…"
        } else {
          "${formatBytes(transfer.done)} of ${formatBytes(transfer.total)} · ${formatSpeed(transfer.bytesPerSec)}"
        },
        style = MaterialTheme.typography.bodySmall,
      )
    }
  }
}

@Composable
private fun HistoryRow(record: TransferRecord) {
  val direction = if (record.direction == Direction.SENT) "Sent to" else "Received from"
  val status =
    when (record.status) {
      TransferStatus.COMPLETED -> null
      TransferStatus.FAILED -> "failed"
      TransferStatus.IN_PROGRESS -> "unfinished"
    }
  ListItem(
    headlineContent = { Text(record.fileName, maxLines = 1, overflow = TextOverflow.Ellipsis) },
    supportingContent = {
      Column {
        Text(
          listOfNotNull("$direction ${record.deviceName}", formatBytes(record.fileSize.toLong()), status, timeAgo(record.createdAt.toLong()))
            .joinToString(" · ")
        )
        record.error?.let { Text(it, color = MaterialTheme.colorScheme.error, maxLines = 2, overflow = TextOverflow.Ellipsis) }
      }
    },
  )
}

@Preview(showBackground = true)
@Composable
private fun TransfersScreenPreview() {
  TOVITheme {
    TransfersScreen(
      active = listOf(ActiveTransfer("1", Direction.SENT, "Office PC", "holiday.mp4", 2_000_000_000, 640_000_000, 48_000_000)),
      history =
        listOf(
          TransferRecord("2", "notes.pdf", 1_200_000u, Direction.RECEIVED, "x", "MacBook", TransferStatus.COMPLETED, 1_700_000_000u, null, null, null),
          TransferRecord("3", "big.iso", 4_000_000_000u, Direction.SENT, "y", "Office PC", TransferStatus.FAILED, 1_700_000_000u, null, null, "declined"),
        ),
    )
  }
}
