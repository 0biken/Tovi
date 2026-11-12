package app.tovi.android

import android.app.Application
import app.tovi.android.data.MulticastLockHolder
import app.tovi.android.data.NodeRepository
import app.tovi.android.data.OutgoingFiles
import app.tovi.android.data.wifiMulticastLock
import app.tovi.android.service.SUMMARY_SETTLE_MS
import app.tovi.android.service.TransferService
import app.tovi.android.service.shouldPostSummary
import app.tovi.android.service.summaryText
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.combine
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
    // Discovery needs multicast: hold the lock while the app is shown or a send runs
    val multicastLock = MulticastLockHolder(wifiMulticastLock(this))
    appScope.launch {
      combine(AppVisibility.visible, repository.queue) { visible, queue -> visible to queue.isActive }
        .collect { (visible, sending) -> multicastLock.update(visible, sending) }
    }
    // batches has no replay: collect from process start, or a summary could be lost
    appScope.launch {
      repository.batches.collect { batch ->
        if (batch.isEmpty()) return@collect
        // Its own coroutine, so waiting never delays the next batch
        launch {
          // Right after the app is closed it still reads as visible; let that settle
          delay(SUMMARY_SETTLE_MS)
          if (shouldPostSummary(AppVisibility.visible.value)) {
            TransferService.postSummary(this@ToviApp, summaryText(batch))
          }
        }
      }
    }
  }
}
