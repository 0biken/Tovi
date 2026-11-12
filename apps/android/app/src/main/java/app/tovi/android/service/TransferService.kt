package app.tovi.android.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import android.os.IBinder
import android.util.Log
import app.tovi.android.share.isShareableUri
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import app.tovi.android.MainActivity
import app.tovi.android.ToviApp
import app.tovi.android.data.ActiveTransfer
import app.tovi.android.data.NodeRepository
import app.tovi.core.Direction
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.conflate
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * Keeps the process alive while the send queue has work, showing progress.
 * It stops itself, and removes its notification, once the queue is empty.
 */
class TransferService : Service() {
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
  private var observer: Job? = null
  private var updates: Job? = null
  /** The newest start request delivered; stopping for an older one would cut off a pending start */
  private var lastStartId = 0
  private lateinit var repository: NodeRepository

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    lastStartId = startId
    repository = (application as ToviApp).repository
    ensureChannels(this)
    // Straight away: the system kills the app if this is late
    ServiceCompat.startForeground(
      this,
      ID_PROGRESS,
      progressNotification(this, progressText(repository.queue.value, currentSend())),
      ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
    )

    val shared = intent?.sharedUris().orEmpty()
    val deviceId = intent?.getStringExtra(EXTRA_DEVICE_ID)
    val deviceName = intent?.getStringExtra(EXTRA_DEVICE_NAME)
    val enqueueShared: suspend () -> Unit = {
      if (shared.isNotEmpty() && deviceId != null) repository.sendNow(deviceId, deviceName ?: deviceId.take(8), shared)
    }
    if (observer?.isActive == true) {
      scope.launch { enqueueShared() }
    } else {
      // Shared files are queued first: until then the queue is empty and would stop the service
      observer =
        scope.launch {
          enqueueShared()
          observe()
        }
    }
    return START_NOT_STICKY
  }

  private suspend fun observe() {
    updates =
      scope.launch {
        combine(repository.queue, repository.live) { queue, _ -> progressText(queue, currentSend()) }
          .conflate()
          .collect { text ->
            notifyProgress(text)
            delay(UPDATE_INTERVAL_MS) // at most two updates a second
          }
      }
    while (true) {
      repository.queue.first { !it.isActive }
      // The queue may have started again since (a flip StateFlow can hide from the edge hook)
      if (repository.queue.value.isActive) continue
      break
    }
    updates?.cancel() // a late update would bring the notification back
    // False when a newer start is still to be delivered: it calls startForeground
    // again and starts a fresh observer, so this one just ends and the notification stays
    if (stopSelfResult(lastStartId)) {
      ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
    }
  }

  private fun currentSend(): ActiveTransfer? =
    repository.live.value.transfers.values.firstOrNull { it.direction == Direction.SENT }

  private fun notifyProgress(text: ProgressText) {
    getSystemService(NotificationManager::class.java).notify(ID_PROGRESS, progressNotification(this, text))
  }

  /** The system's time limit for a dataSync service (Android 15+) */
  override fun onTimeout(startId: Int, fgsType: Int) {
    Log.w(TAG, "foreground service timed out")
    updates?.cancel()
    ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
    stopSelf()
  }

  override fun onDestroy() {
    scope.cancel()
    super.onDestroy()
  }

  companion object {
    private const val TAG = "Tovi"
    const val CHANNEL_PROGRESS = "transfers"
    const val CHANNEL_RESULTS = "transfer_results"
    const val ID_PROGRESS = 1
    const val ID_RESULT = 2
    const val EXTRA_OPEN = "open"
    const val OPEN_TRANSFERS = "transfers"
    private const val EXTRA_DEVICE_ID = "device_id"
    private const val EXTRA_DEVICE_NAME = "device_name"
    private const val UPDATE_INTERVAL_MS = 500L

    /**
     * Start the service. With [shared] it also queues those files for
     * [deviceId]; the grant on the intent keeps them readable until it stops.
     * Returns false when the system refused to start it.
     */
    fun start(context: Context, shared: List<Uri> = emptyList(), deviceId: String? = null, deviceName: String? = null): Boolean {
      val intent = Intent(context, TransferService::class.java)
      if (shared.isNotEmpty()) {
        val clip = ClipData.newRawUri("shared", shared.first())
        shared.drop(1).forEach { clip.addItem(ClipData.Item(it)) }
        intent.clipData = clip
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        intent.putExtra(EXTRA_DEVICE_ID, deviceId)
        intent.putExtra(EXTRA_DEVICE_NAME, deviceName)
      }
      try {
        ContextCompat.startForegroundService(context, intent)
        return true
      } catch (e: RuntimeException) {
        // Not allowed to start from the background, or the grant was refused; the caller decides what to tell the user
        Log.w(TAG, "could not start the transfer service", e)
        return false
      }
    }

    private fun Intent.sharedUris(): List<Uri> =
      clipData?.let { clip -> (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }.filter { isShareableUri(it.scheme) } }.orEmpty()

    /** Idempotent: creating an existing channel changes nothing */
    fun ensureChannels(context: Context) {
      val manager = context.getSystemService(NotificationManager::class.java)
      manager.createNotificationChannel(NotificationChannel(CHANNEL_PROGRESS, "Transfers", NotificationManager.IMPORTANCE_LOW))
      manager.createNotificationChannel(
        NotificationChannel(CHANNEL_RESULTS, "Transfer results", NotificationManager.IMPORTANCE_DEFAULT)
      )
    }

    private fun openTransfers(context: Context): PendingIntent =
      PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java)
          .putExtra(EXTRA_OPEN, OPEN_TRANSFERS)
          .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
      )

    private fun progressNotification(context: Context, text: ProgressText): Notification =
      NotificationCompat.Builder(context, CHANNEL_PROGRESS)
        .setSmallIcon(android.R.drawable.stat_sys_upload)
        .setContentTitle(text.title)
        .setContentText(text.body)
        .setProgress(100, text.percent ?: 0, text.percent == null)
        .setOngoing(true)
        .setOnlyAlertOnce(true)
        .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
        .setContentIntent(openTransfers(context))
        .build()

    /** Posts the end-of-run notification; without the notification permission the system drops it */
    fun postSummary(context: Context, text: SummaryText) {
      ensureChannels(context)
      val notification =
        NotificationCompat.Builder(context, CHANNEL_RESULTS)
          .setSmallIcon(android.R.drawable.stat_sys_upload_done)
          .setContentTitle(text.title)
          .setContentText(text.body)
          .setStyle(NotificationCompat.BigTextStyle().bigText(text.body ?: text.title))
          .setAutoCancel(true)
          .setContentIntent(openTransfers(context))
          .build()
      context.getSystemService(NotificationManager::class.java).notify(ID_RESULT, notification)
    }
  }
}
