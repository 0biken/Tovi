package app.tovi.android.share

import android.Manifest
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.foundation.clickable
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.tovi.android.KEY_ASKED
import app.tovi.android.MainActivity
import app.tovi.android.ToviApp
import app.tovi.android.UI_PREFS
import app.tovi.android.service.TransferService
import app.tovi.android.shouldAskNotifications
import app.tovi.android.theme.TOVITheme
import app.tovi.core.Device

/** Share → TOVI: pick which paired device receives the shared files */
class ShareActivity : ComponentActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val uris = streamsOf(intent)
    if (uris.isEmpty()) {
      finish()
      return
    }
    val repository = (application as ToviApp).repository
    val title = if (uris.size == 1) "Send ${displayName(uris[0])} to…" else "Send ${uris.size} files to…"

    setContent {
      TOVITheme {
        val devices by repository.devices.collectAsStateWithLifecycle()
        val chosen = remember { mutableStateOf<Device?>(null) }

        fun send(device: Device) {
          TransferService.start(this, uris, device.id, device.name)
          finish()
        }

        // Asked once, before the first send; a refusal never blocks it
        val askNotifications =
          rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
            chosen.value?.let(::send)
          }

        Surface(color = MaterialTheme.colorScheme.background) {
          if (devices.isEmpty()) {
            NoDevices(onOpen = {
              startActivity(Intent(this, MainActivity::class.java))
              finish()
            })
          } else {
            DevicePicker(title, devices) { device ->
              chosen.value = device
              if (shouldAskNotifications(this)) {
                getSharedPreferences(UI_PREFS, Context.MODE_PRIVATE).edit().putBoolean(KEY_ASKED, true).apply()
                askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
              } else {
                send(device)
              }
            }
          }
        }
      }
    }
  }

  private fun streamsOf(intent: Intent): List<Uri> =
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      sharedStreams(
        intent.action,
        intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java),
        intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java),
      )
    } else {
      @Suppress("DEPRECATION")
      sharedStreams(
        intent.action,
        intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM),
        intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM),
      )
    }

  private fun displayName(uri: Uri): String =
    runCatching {
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
          if (c.moveToFirst()) c.getString(0) else null
        }
      }
      .getOrNull() ?: uri.lastPathSegment ?: "file"
}

@Composable
private fun DevicePicker(title: String, devices: List<Device>, onChoose: (Device) -> Unit) {
  Column(Modifier.padding(vertical = 16.dp)) {
    Text(title, style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
    LazyColumn {
      items(devices, key = { it.id }) { device ->
        ListItem(headlineContent = { Text(device.name) }, modifier = Modifier.clickable { onChoose(device) })
      }
    }
  }
}

@Composable
private fun NoDevices(onOpen: () -> Unit) {
  Column(
    Modifier.fillMaxWidth().padding(24.dp),
    verticalArrangement = Arrangement.spacedBy(16.dp),
  ) {
    Text("Pair a device in TOVI first", style = MaterialTheme.typography.titleLarge)
    Button(onClick = onOpen) { Text("Open TOVI") }
  }
}
