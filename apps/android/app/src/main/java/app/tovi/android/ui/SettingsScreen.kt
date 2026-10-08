package app.tovi.android.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ListItem
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import app.tovi.android.data.NodeRepository.Status
import app.tovi.android.theme.TOVITheme
import app.tovi.core.LocalId

@Composable
fun SettingsScreen(
  status: Status,
  autoAccept: Boolean,
  receiveDir: String,
  onAutoAcceptChange: (Boolean) -> Unit,
  modifier: Modifier = Modifier,
) {
  Column(modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
    ListItem(
      headlineContent = { Text("Save files without asking") },
      supportingContent = { Text("Files from paired devices are saved straight away. Turn off to approve each one.") },
      trailingContent = { Switch(checked = autoAccept, onCheckedChange = onAutoAcceptChange, enabled = status is Status.Running) },
    )
    HorizontalDivider()
    ListItem(
      headlineContent = { Text("Received files") },
      supportingContent = { SelectionContainer { Text(receiveDir.ifEmpty { "…" }) } },
    )
    HorizontalDivider()
    if (status is Status.Running) {
      ListItem(headlineContent = { Text("Device name") }, supportingContent = { Text(status.local.deviceName) })
      ListItem(
        headlineContent = { Text("Device ID") },
        supportingContent = { SelectionContainer { Text(status.local.deviceId) } },
      )
      ListItem(headlineContent = { Text("Port") }, supportingContent = { Text("UDP ${status.port}") })
    }
  }
}

@Preview(showBackground = true)
@Composable
private fun SettingsScreenPreview() {
  TOVITheme {
    SettingsScreen(
      status = Status.Running(LocalId("ab".repeat(32), "Pixel 9"), 48210),
      autoAccept = true,
      receiveDir = "/storage/emulated/0/Android/data/app.tovi.android/files/Download/TOVI",
      onAutoAcceptChange = {},
    )
  }
}
