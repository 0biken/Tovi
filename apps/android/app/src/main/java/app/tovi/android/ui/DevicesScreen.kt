package app.tovi.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import app.tovi.android.data.NodeRepository.Status
import app.tovi.android.theme.TOVITheme
import app.tovi.core.Device

@Composable
fun DevicesScreen(
  status: Status,
  devices: List<Device>,
  onPair: () -> Unit,
  /** Open the file picker to send to this device */
  onSend: (deviceId: String) -> Unit,
  onForget: (deviceId: String) -> Unit,
  modifier: Modifier = Modifier,
) {
  var forgetting by remember { mutableStateOf<Device?>(null) }

  Box(modifier.fillMaxSize()) {
    // Bottom padding keeps the last device clear of the button
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 88.dp)) {
      item { StatusHeader(status) }
      if (devices.isEmpty()) {
        item {
          Text(
            "No paired devices yet. Open TOVI on your computer, go to Pair, then tap Pair device here.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth().padding(32.dp),
          )
        }
      }
      items(devices, key = { it.id }) { device ->
        ListItem(
          headlineContent = { Text(device.name) },
          supportingContent = {
            val seen = device.lastSeen?.let { " · seen ${timeAgo(it.toLong())}" } ?: ""
            Text(platformLabel(device.platform) + seen)
          },
          trailingContent = {
            Row {
              IconButton(onClick = { onSend(device.id) }) {
                Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Send a file to ${device.name}")
              }
              IconButton(onClick = { forgetting = device }) {
                Icon(Icons.Default.Delete, contentDescription = "Forget ${device.name}")
              }
            }
          },
        )
        HorizontalDivider()
      }
    }
    ExtendedFloatingActionButton(
      onClick = onPair,
      icon = { Icon(Icons.Default.Add, contentDescription = null) },
      text = { Text("Pair device") },
      modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
    )
  }

  forgetting?.let { device ->
    AlertDialog(
      onDismissRequest = { forgetting = null },
      title = { Text("Forget ${device.name}?") },
      text = { Text("It won't be able to send you files until you pair again.") },
      confirmButton = {
        TextButton(
          onClick = {
            onForget(device.id)
            forgetting = null
          }
        ) {
          Text("Forget")
        }
      },
      dismissButton = { TextButton(onClick = { forgetting = null }) { Text("Cancel") } },
    )
  }
}

@Composable
private fun StatusHeader(status: Status) {
  Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
    when (status) {
      Status.Starting -> Text("Starting…", style = MaterialTheme.typography.titleMedium)
      is Status.Running -> {
        Text(status.local.deviceName, style = MaterialTheme.typography.titleMedium)
        Text(
          "Ready to receive · ID ${status.local.deviceId.take(8)}",
          style = MaterialTheme.typography.bodySmall,
          color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
      }
      is Status.Failed -> {
        Text("TOVI could not start", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.error)
        Text(status.message, style = MaterialTheme.typography.bodySmall)
      }
    }
  }
}

@Preview(showBackground = true)
@Composable
private fun DevicesScreenPreview() {
  TOVITheme {
    DevicesScreen(
      status = Status.Starting,
      devices =
        listOf(
          Device("a".repeat(64), "Office PC", "windows", 0u, null, "192.168.1.20:48210"),
          Device("b".repeat(64), "MacBook", "macos", 0u, 1_700_000_000u, null),
        ),
      onPair = {},
      onSend = {},
      onForget = {},
    )
  }
}
