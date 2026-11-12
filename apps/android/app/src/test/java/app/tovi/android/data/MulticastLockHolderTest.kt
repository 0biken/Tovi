package app.tovi.android.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MulticastLockHolderTest {
  private class FakeLock : LockHandle {
    var acquires = 0
    var releases = 0
    override var isHeld = false
      private set

    override fun acquire() {
      acquires++
      isHeld = true
    }

    override fun release() {
      releases++
      isHeld = false
    }
  }

  @Test
  fun heldWhileVisible() {
    val lock = FakeLock()
    val holder = MulticastLockHolder(lock)
    holder.update(appVisible = true, sending = false)
    assertTrue(lock.isHeld)
    holder.update(appVisible = false, sending = false)
    assertFalse(lock.isHeld)
  }

  @Test
  fun heldWhileSending() {
    val lock = FakeLock()
    val holder = MulticastLockHolder(lock)
    holder.update(appVisible = false, sending = true)
    assertTrue(lock.isHeld)
    holder.update(appVisible = false, sending = false)
    assertFalse(lock.isHeld)
  }

  @Test
  fun lockHeldUntilQueueEmptiesAfterBackgrounding() {
    val lock = FakeLock()
    val holder = MulticastLockHolder(lock)
    holder.update(appVisible = true, sending = false)
    holder.update(appVisible = true, sending = true)
    holder.update(appVisible = false, sending = true)
    assertTrue(lock.isHeld)
    holder.update(appVisible = false, sending = false)
    assertFalse(lock.isHeld)
    assertEquals(1, lock.acquires)
    assertEquals(1, lock.releases)
  }

  @Test
  fun repeatedUpdatesDoNotReacquire() {
    val lock = FakeLock()
    val holder = MulticastLockHolder(lock)
    repeat(3) { holder.update(appVisible = true, sending = false) }
    assertEquals(1, lock.acquires)
    repeat(3) { holder.update(appVisible = false, sending = false) }
    assertEquals(1, lock.releases)
  }
}
