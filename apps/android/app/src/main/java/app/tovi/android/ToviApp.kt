package app.tovi.android

import android.app.Application
import app.tovi.android.data.NodeRepository
import app.tovi.android.data.OutgoingFiles
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

class ToviApp : Application() {
  /** Lives as long as the process; so does the node */
  private val appScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

  lateinit var repository: NodeRepository
    private set

  override fun onCreate() {
    super.onCreate()
    // Nothing can be sending yet: copies left over are from a process that died
    OutgoingFiles.clearCache(this)
    repository = NodeRepository(this, appScope).also { it.start() }
  }
}
