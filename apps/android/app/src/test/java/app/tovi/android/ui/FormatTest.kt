package app.tovi.android.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class FormatTest {
  @Test
  fun bytesMatchTheDesktopApp() {
    assertEquals("999 B", formatBytes(999))
    assertEquals("2 KB", formatBytes(1_500))
    assertEquals("1.5 MB", formatBytes(1_500_000))
    assertEquals("2.0 GB", formatBytes(2_000_000_000))
    assertEquals("48.0 MB/s", formatSpeed(48_000_000))
  }

  @Test
  fun timeAgo() {
    val now = 1_700_000_000L
    assertEquals("just now", timeAgo(now - 30, now))
    assertEquals("just now", timeAgo(now + 30, now))
    assertEquals("5 min ago", timeAgo(now - 300, now))
    assertEquals("3 h ago", timeAgo(now - 3 * 3600, now))
    assertEquals("1 day ago", timeAgo(now - 86_400, now))
    assertEquals("2 days ago", timeAgo(now - 2 * 86_400, now))
  }
}
