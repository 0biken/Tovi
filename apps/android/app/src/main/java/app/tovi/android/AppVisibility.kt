package app.tovi.android

import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Whether any of the app's activities is on screen; fed by the process lifecycle */
object AppVisibility {
  private val _visible = MutableStateFlow(false)
  val visible: StateFlow<Boolean> = _visible.asStateFlow()

  /** Call once, on the main thread, from [ToviApp.onCreate] */
  fun track() {
    ProcessLifecycleOwner.get()
      .lifecycle
      .addObserver(
        object : DefaultLifecycleObserver {
          override fun onStart(owner: LifecycleOwner) {
            _visible.value = true
          }

          override fun onStop(owner: LifecycleOwner) {
            _visible.value = false
          }
        }
      )
  }
}
