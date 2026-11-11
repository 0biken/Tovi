package app.tovi.android.ui

import java.util.Locale

// Same output as the desktop app's src/format.ts

fun formatBytes(bytes: Long): String =
  when {
    bytes >= 1_000_000_000 -> String.format(Locale.ROOT, "%.1f GB", bytes / 1e9)
    bytes >= 1_000_000 -> String.format(Locale.ROOT, "%.1f MB", bytes / 1e6)
    bytes >= 1_000 -> String.format(Locale.ROOT, "%.0f KB", bytes / 1e3)
    else -> "$bytes B"
  }

fun formatSpeed(bytesPerSec: Long): String = "${formatBytes(bytesPerSec)}/s"

/** "just now", "5 min ago", "3 h ago", "2 days ago" */
fun timeAgo(unixSecs: Long, nowSecs: Long = System.currentTimeMillis() / 1000): String {
  val s = (nowSecs - unixSecs).coerceAtLeast(0)
  return when {
    s < 60 -> "just now"
    s < 3600 -> "${s / 60} min ago"
    s < 86400 -> "${s / 3600} h ago"
    else -> {
      val days = s / 86400
      "$days day${if (days == 1L) "" else "s"} ago"
    }
  }
}

fun platformLabel(platform: String): String =
  when (platform) {
    "macos" -> "macOS"
    "windows" -> "Windows"
    "linux" -> "Linux"
    "android" -> "Android"
    "ios" -> "iOS"
    else -> platform
  }
