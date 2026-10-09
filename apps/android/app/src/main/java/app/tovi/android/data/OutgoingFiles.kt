package app.tovi.android.data

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import java.io.File
import java.io.FileNotFoundException
import java.io.IOException

/** Copies picked or shared `content://` documents to files the core can read by path */
object OutgoingFiles {
  /**
   * Copy [uri] to `cache/outgoing/<n>/<display name>`, so the receiver sees
   * the original file name. Throws if it can't be read or doesn't fit.
   */
  fun copyToCache(context: Context, uri: Uri): File {
    val resolver = context.contentResolver
    var name: String? = null
    var size: Long? = null
    resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { cursor ->
      if (cursor.moveToFirst()) {
        cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME).takeIf { it >= 0 }?.let { name = cursor.getString(it) }
        cursor.getColumnIndex(OpenableColumns.SIZE).takeIf { it >= 0 && !cursor.isNull(it) }?.let { size = cursor.getLong(it) }
      }
    }
    val dir = File(context.cacheDir, "outgoing/${System.nanoTime()}").apply { mkdirs() }
    size?.let { if (it > dir.usableSpace) throw IOException("not enough free space to prepare it") }
    val target = File(dir, safeName(name ?: uri.lastPathSegment ?: "file"))
    val input = resolver.openInputStream(uri) ?: throw FileNotFoundException(uri.toString())
    input.use { source -> target.outputStream().use { source.copyTo(it, bufferSize = 1 shl 20) } }
    return target
  }

  /** Keep only the last path component; the receiver sanitizes it again */
  fun safeName(raw: String): String = raw.substringAfterLast('/').substringAfterLast('\\').trim().ifEmpty { "file" }

  /** Remove copies left by sends the process didn't live to finish */
  fun clearCache(context: Context) {
    File(context.cacheDir, "outgoing").deleteRecursively()
  }
}
