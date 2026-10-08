package app.tovi.android.ui

import android.net.Uri
import android.util.Log
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider.AndroidViewModelFactory.Companion.APPLICATION_KEY
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.createSavedStateHandle
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import app.tovi.android.ToviApp
import app.tovi.android.data.NodeRepository
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

sealed interface PairState {
  data object Idle : PairState

  data object Pairing : PairState

  data class Paired(val deviceName: String) : PairState

  data class Failed(val message: String) : PairState
}

/** The screens' view of [NodeRepository], plus the pairing screen's state */
class AppViewModel(private val repository: NodeRepository, private val saved: SavedStateHandle) : ViewModel() {
  val status = repository.status
  val devices = repository.devices
  val history = repository.history
  val live = repository.live
  val autoAccept = repository.autoAccept
  val receiveDir = repository.receiveDir
  val notices = repository.notices

  private val _pairState = MutableStateFlow<PairState>(PairState.Idle)
  val pairState: StateFlow<PairState> = _pairState.asStateFlow()

  fun pair(code: String) {
    if (_pairState.value == PairState.Pairing) return
    _pairState.value = PairState.Pairing
    viewModelScope.launch {
      _pairState.value =
        repository
          .pair(code.trim())
          .fold({ PairState.Paired(it.name) }, { PairState.Failed(it.message ?: "Pairing failed") })
    }
  }

  fun resetPairing() {
    _pairState.value = PairState.Idle
  }

  fun respond(requestId: ULong, allow: Boolean) = repository.respond(requestId, allow)

  /**
   * Remember which device the file picker is choosing for. Kept in saved
   * state: Android may kill the app while the picker is open and deliver the
   * result to a new process.
   */
  fun startPicking(deviceId: String) {
    saved[PICKING_FOR] = deviceId
  }

  /** The picker's result; null when the user backed out */
  fun filePicked(uri: Uri?) {
    val deviceId = saved.get<String>(PICKING_FOR)
    saved.remove<String>(PICKING_FOR)
    when {
      uri == null -> Unit
      deviceId == null -> Log.w("Tovi", "a file was picked but the device it was for was lost")
      else -> repository.send(deviceId, uri)
    }
  }

  fun forget(deviceId: String) = repository.forget(deviceId)

  fun setAutoAccept(enabled: Boolean) = repository.setAutoAccept(enabled)

  companion object {
    private const val PICKING_FOR = "picking_for"

    val Factory = viewModelFactory {
      initializer { AppViewModel((this[APPLICATION_KEY] as ToviApp).repository, createSavedStateHandle()) }
    }
  }
}
