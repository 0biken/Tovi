package app.tovi.android

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.tovi.android.service.TransferService
import app.tovi.android.theme.TOVITheme
import app.tovi.android.ui.PairingLink
import kotlinx.coroutines.flow.MutableStateFlow

class MainActivity : ComponentActivity() {
  /** A `tovi://pair/...` link this activity was opened with, until the UI shows it */
  private val pairLink = MutableStateFlow<String?>(null)

  /** Set when opened from a notification, until the UI has switched to Transfers */
  private val openTransfers = MutableStateFlow(false)

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    // On recreation the link was already shown and its code kept in saved state
    if (savedInstanceState == null) {
      takePairLink(intent)
      takeOpenRequest(intent)
    }

    enableEdgeToEdge()
    setContent {
      val link by pairLink.collectAsStateWithLifecycle()
      val showTransfers by openTransfers.collectAsStateWithLifecycle()
      TOVITheme {
        MainNavigation(
          pairLink = link,
          onPairLinkShown = { pairLink.value = null },
          openTransfers = showTransfers,
          onTransfersOpened = { openTransfers.value = false },
        )
      }
    }
  }

  /** singleTop: a link opened while TOVI is already showing arrives here */
  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    takePairLink(intent)
    takeOpenRequest(intent)
  }

  /** A tap on a transfer notification asks for the Transfers tab */
  private fun takeOpenRequest(intent: Intent?) {
    if (intent?.getStringExtra(TransferService.EXTRA_OPEN) == TransferService.OPEN_TRANSFERS) openTransfers.value = true
  }

  private fun takePairLink(intent: Intent?) {
    if (intent?.action == Intent.ACTION_VIEW) PairingLink.parse(intent.dataString)?.let { pairLink.value = it }
  }
}
