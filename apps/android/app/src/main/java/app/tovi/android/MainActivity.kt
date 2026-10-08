package app.tovi.android

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.tovi.android.theme.TOVITheme
import app.tovi.android.ui.PairingLink
import kotlinx.coroutines.flow.MutableStateFlow

class MainActivity : ComponentActivity() {
  /** A `tovi://pair/...` link this activity was opened with, until the UI shows it */
  private val pairLink = MutableStateFlow<String?>(null)

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    // On recreation the link was already shown and its code kept in saved state
    if (savedInstanceState == null) takePairLink(intent)

    enableEdgeToEdge()
    setContent {
      val link by pairLink.collectAsStateWithLifecycle()
      TOVITheme { MainNavigation(pairLink = link, onPairLinkShown = { pairLink.value = null }) }
    }
  }

  /** singleTop: a link opened while TOVI is already showing arrives here */
  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    takePairLink(intent)
  }

  private fun takePairLink(intent: Intent?) {
    if (intent?.action == Intent.ACTION_VIEW) PairingLink.parse(intent.dataString)?.let { pairLink.value = it }
  }
}
