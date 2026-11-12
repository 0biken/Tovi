package app.tovi.android.data

import android.content.Context
import android.net.wifi.WifiManager
import android.util.Log

/** Something that can be held and let go of, so the rule below is testable without Android */
interface LockHandle {
  fun acquire()

  fun release()

  val isHeld: Boolean
}

/**
 * Android drops multicast packets unless a lock is held, and discovery needs them.
 * Held while the app is on screen or a send is running, so it never drains the
 * battery of an idle phone.
 */
class MulticastLockHolder(private val lock: LockHandle) {
  /** Held iff either is true; calling again with the same values does nothing */
  @Synchronized
  fun update(appVisible: Boolean, sending: Boolean) {
    val wanted = appVisible || sending
    if (wanted && !lock.isHeld) lock.acquire()
    if (!wanted && lock.isHeld) lock.release()
  }
}

/** A lock that is not reference-counted, so one release always lets go. Does nothing if there is no Wi-Fi service. */
fun wifiMulticastLock(context: Context): LockHandle {
  val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
  val lock = wifi?.createMulticastLock("tovi")?.apply { setReferenceCounted(false) }
  return object : LockHandle {
    // A SecurityException (say, the permission was revoked) must not end the collector that calls these
    override fun acquire() {
      runCatching { lock?.acquire() }.onFailure { Log.w("Tovi", "could not hold the multicast lock", it) }
    }

    override fun release() {
      runCatching { lock?.release() }.onFailure { Log.w("Tovi", "could not release the multicast lock", it) }
    }

    override val isHeld: Boolean get() = lock?.isHeld ?: false
  }
}
