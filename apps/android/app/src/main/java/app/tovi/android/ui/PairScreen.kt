package app.tovi.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import app.tovi.android.theme.TOVITheme

const val PAIRING_LINK_PREFIX = "tovi://pair/"

/** Pair by pasting the computer's pairing code. The camera scanner joins this screen next. */
@Composable
fun PairScreen(state: PairState, onPair: (code: String) -> Unit, onPaired: () -> Unit, modifier: Modifier = Modifier) {
  var code by rememberSaveable { mutableStateOf("") }
  val pairing = state == PairState.Pairing
  val looksValid = code.trim().startsWith(PAIRING_LINK_PREFIX)

  LaunchedEffect(state) { if (state is PairState.Paired) onPaired() }

  Column(modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
    Text(
      "On your computer, open TOVI and go to Pair. Click Copy code and paste the code here. A new code appears every 60 seconds.",
      style = MaterialTheme.typography.bodyMedium,
    )
    OutlinedTextField(
      value = code,
      onValueChange = { code = it },
      label = { Text("Pairing code") },
      placeholder = { Text("${PAIRING_LINK_PREFIX}…") },
      singleLine = true,
      enabled = !pairing,
      isError = code.isNotBlank() && !looksValid,
      supportingText = { if (code.isNotBlank() && !looksValid) Text("A pairing code starts with $PAIRING_LINK_PREFIX") },
      keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
      keyboardActions = KeyboardActions(onGo = { if (looksValid) onPair(code) }),
      modifier = Modifier.fillMaxWidth(),
    )
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
      Button(onClick = { onPair(code) }, enabled = looksValid && !pairing) { Text("Pair") }
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

@Preview(showBackground = true)
@Composable
private fun PairScreenPreview() {
  TOVITheme { PairScreen(PairState.Failed("the pairing code has expired"), onPair = {}, onPaired = {}) }
}
