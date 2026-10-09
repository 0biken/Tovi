package app.tovi.android.ui

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import app.tovi.android.theme.TOVITheme

/** Pair by scanning the computer's QR code, or by pasting its code */
@Composable
fun PairScreen(
  state: PairState,
  code: String,
  codeFromLink: Boolean,
  onCodeChange: (String) -> Unit,
  onScanned: (String) -> Unit,
  onPair: (String) -> Unit,
  onPaired: () -> Unit,
  modifier: Modifier = Modifier,
) {
  val pairing = state == PairState.Pairing
  val link = PairingLink.parse(code)

  LaunchedEffect(state) { if (state is PairState.Paired) onPaired() }

  Column(
    modifier.verticalScroll(rememberScrollState()).padding(16.dp),
    verticalArrangement = Arrangement.spacedBy(16.dp),
  ) {
    // A code from a link goes straight to the confirmation below; no need to scan
    if (!codeFromLink) {
      Text(
        "On your computer, open TOVI and go to Pair. Point the camera at the QR code it shows.",
        style = MaterialTheme.typography.bodyMedium,
      )
      CameraArea(onScanned = onScanned, scanning = !pairing)
    }

    if (codeFromLink) {
      Text(
        "A pairing link was opened. Only pair if it came from your own computer: a paired device can send you files.",
        style = MaterialTheme.typography.bodyMedium,
      )
    } else {
      Text("Or paste the code (Copy code on the computer):", style = MaterialTheme.typography.bodyMedium)
    }
    OutlinedTextField(
      value = code,
      onValueChange = onCodeChange,
      label = { Text("Pairing code") },
      placeholder = { Text("${PairingLink.PREFIX}…") },
      singleLine = true,
      enabled = !pairing,
      isError = code.isNotBlank() && link == null,
      supportingText = { if (code.isNotBlank() && link == null) Text("A pairing code starts with ${PairingLink.PREFIX}") },
      keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
      keyboardActions = KeyboardActions(onGo = { link?.let(onPair) }),
      modifier = Modifier.fillMaxWidth(),
    )
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
      Button(onClick = { link?.let(onPair) }, enabled = link != null && !pairing) { Text("Pair") }
      if (pairing) {
        CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
        Text("Waiting for the other device to allow it…", style = MaterialTheme.typography.bodySmall)
      }
    }
    if (state is PairState.Failed) {
      Text(state.message, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
    }
  }
}

/** The scanner, or what's needed to get it: camera permission, or a camera at all */
@Composable
private fun CameraArea(onScanned: (String) -> Unit, scanning: Boolean) {
  val context = LocalContext.current
  val hasCamera = remember { context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY) }
  var granted by remember { mutableStateOf(cameraGranted(context)) }
  // Asked once per visit; after that the button asks again
  var asked by rememberSaveable { mutableStateOf(false) }
  val requestCamera = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted = it }

  // Permission may be granted in system settings while we're away
  LifecycleResumeEffect(Unit) {
    granted = cameraGranted(context)
    onPauseOrDispose {}
  }
  LaunchedEffect(hasCamera) {
    if (hasCamera && !granted && !asked) {
      asked = true
      requestCamera.launch(Manifest.permission.CAMERA)
    }
  }

  val shape = RoundedCornerShape(16.dp)
  when {
    !hasCamera -> Unit
    granted ->
      Box(Modifier.fillMaxWidth().aspectRatio(1f).clip(shape)) {
        QrScanner(onLink = { if (scanning) onScanned(it) }, modifier = Modifier.fillMaxWidth().aspectRatio(1f))
      }
    else ->
      Surface(shape = shape, color = MaterialTheme.colorScheme.surfaceVariant, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
          Text(
            "TOVI needs the camera to scan the code. It's only used on this screen.",
            style = MaterialTheme.typography.bodyMedium,
            textAlign = TextAlign.Start,
          )
          Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = { requestCamera.launch(Manifest.permission.CAMERA) }) { Text("Allow camera") }
            // Once refused for good, only system settings can grant it
            TextButton(onClick = { context.startActivity(appSettings(context)) }) { Text("App settings") }
          }
        }
      }
  }
}

private fun cameraGranted(context: Context) =
  ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED

private fun appSettings(context: Context) =
  Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null))
    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

@Preview(showBackground = true)
@Composable
private fun PairScreenFromLinkPreview() {
  TOVITheme {
    PairScreen(
      state = PairState.Failed("the pairing code has expired"),
      code = "${PairingLink.PREFIX}pWF2AWFr",
      codeFromLink = true,
      onCodeChange = {},
      onScanned = {},
      onPair = {},
      onPaired = {},
    )
  }
}
