package app.tovi.android

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation3.runtime.NavBackStack
import androidx.navigation3.runtime.NavKey
import androidx.navigation3.runtime.entryProvider
import androidx.navigation3.runtime.rememberNavBackStack
import androidx.navigation3.ui.NavDisplay
import app.tovi.android.ui.AppViewModel
import app.tovi.android.ui.DevicesScreen
import app.tovi.android.ui.PairScreen
import app.tovi.android.ui.RequestDialog
import app.tovi.android.ui.SettingsScreen
import app.tovi.android.ui.TransfersScreen

private data class TabItem(val tab: Tab, val label: String, val icon: ImageVector)

private val TABS =
  listOf(
    TabItem(Devices, "Devices", Icons.Default.Home),
    TabItem(Transfers, "Transfers", Icons.AutoMirrored.Filled.List),
    TabItem(SettingsTab, "Settings", Icons.Default.Settings),
  )

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainNavigation(
  /** A `tovi://pair/...` link the app was opened with, not yet shown */
  pairLink: String? = null,
  onPairLinkShown: () -> Unit = {},
  viewModel: AppViewModel = viewModel(factory = AppViewModel.Factory),
) {
  val backStack = rememberNavBackStack(Devices)
  val current = backStack.lastOrNull()

  val status by viewModel.status.collectAsStateWithLifecycle()
  val devices by viewModel.devices.collectAsStateWithLifecycle()
  val history by viewModel.history.collectAsStateWithLifecycle()
  val live by viewModel.live.collectAsStateWithLifecycle()
  val autoAccept by viewModel.autoAccept.collectAsStateWithLifecycle()
  val receiveDir by viewModel.receiveDir.collectAsStateWithLifecycle()
  val pairState by viewModel.pairState.collectAsStateWithLifecycle()
  val pairCode by viewModel.pairCode.collectAsStateWithLifecycle()
  val pairCodeFromLink by viewModel.pairCodeFromLink.collectAsStateWithLifecycle()

  LaunchedEffect(pairLink) {
    if (pairLink != null) {
      viewModel.openPairLink(pairLink)
      backStack.showTab(Devices)
      backStack.add(PairDevice)
      onPairLinkShown()
    }
  }

  // Registered here rather than in a screen, so a result delivered to a
  // recreated activity (the app was killed while the picker was open) still lands
  val pickFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument(), viewModel::filePicked)

  val snackbar = remember { SnackbarHostState() }
  LaunchedEffect(Unit) { viewModel.notices.collect { snackbar.showSnackbar(it) } }

  Scaffold(
    topBar = {
      TopAppBar(
        title = { Text(if (current == PairDevice) "Pair a device" else TABS.firstOrNull { it.tab == current }?.label ?: "TOVI") },
        navigationIcon = {
          if (current !is Tab) {
            IconButton(onClick = { backStack.removeLastOrNull() }) {
              Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
          }
        },
      )
    },
    bottomBar = {
      if (current is Tab) {
        NavigationBar {
          TABS.forEach { item ->
            NavigationBarItem(
              selected = current == item.tab,
              onClick = { backStack.showTab(item.tab) },
              icon = { Icon(item.icon, contentDescription = null) },
              label = { Text(item.label) },
            )
          }
        }
      }
    },
    snackbarHost = { SnackbarHost(snackbar) },
  ) { padding ->
    NavDisplay(
      backStack = backStack,
      onBack = { backStack.removeLastOrNull() },
      modifier = Modifier.padding(padding),
      entryProvider =
        entryProvider {
          entry<Devices> {
            DevicesScreen(
              status = status,
              devices = devices,
              onPair = {
                viewModel.resetPairing()
                backStack.add(PairDevice)
              },
              onSend = { deviceId ->
                viewModel.startPicking(deviceId)
                pickFile.launch(arrayOf("*/*"))
              },
              onForget = viewModel::forget,
            )
          }
          entry<Transfers> { TransfersScreen(active = live.transfers.values.toList(), history = history) }
          entry<SettingsTab> {
            SettingsScreen(
              status = status,
              autoAccept = autoAccept,
              receiveDir = receiveDir,
              onAutoAcceptChange = viewModel::setAutoAccept,
            )
          }
          entry<PairDevice> {
            PairScreen(
              state = pairState,
              code = pairCode,
              codeFromLink = pairCodeFromLink,
              onCodeChange = viewModel::editPairCode,
              onScanned = viewModel::scanned,
              onPair = viewModel::pair,
              onPaired = { backStack.removeLastOrNull() },
            )
          }
        },
    )
  }

  // One request at a time, oldest first
  live.requests.firstOrNull()?.let { RequestDialog(it, onRespond = viewModel::respond) }
}

/** Switch tabs: the tab becomes the only entry, so Back leaves the app */
private fun NavBackStack<NavKey>.showTab(tab: Tab) {
  while (size > 1) removeLastOrNull()
  this[0] = tab
}
