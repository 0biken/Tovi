package app.tovi.android

import android.app.Application
import app.tovi.android.data.NodeRepository
import app.tovi.android.data.OutgoingFiles
import app.tovi.android.service.TransferService
import app.tovi.android.service.shouldPostSummary
import app.tovi.android.service.summaryText
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

class ToviApp : Application() {
  /** Lives as long as the process; so does the node */
  private val appScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

  lateinit var repository: NodeRepository
    private set

  override fun onCreate() {
    super.onCreate()
    // Nothing can be sending yet: copies left over are from a process that died
    OutgoingFiles.clearCache(this)
    AppVisibility.track()
    repository = NodeRepository(this, appScope).also { it.start() }
    // batches has no replay: collect from process start, or a summary could be lost
    appScope.launch {
      repository.batches.collect { batch ->
        if (batch.isNotEmpty() && shouldPostSummary(AppVisibility.visible.value)) {
          TransferService.postSummary(this@ToviApp, summaryText(batch))
        }
      }
    }
  }
}
