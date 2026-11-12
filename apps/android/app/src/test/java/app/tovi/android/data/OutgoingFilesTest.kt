package app.tovi.android.data

import java.io.File
import java.io.IOException
import kotlinx.coroutines.CancellationException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class OutgoingFilesTest {
  private fun tempDir(): File = java.nio.file.Files.createTempDirectory("outgoing").toFile()

  @Test
  fun failedCopyRemovesTheDirectoryAndPartialFile() {
    val dir = tempDir()
    try {
      deletingOnFailure(dir) {
        File(dir, "big.bin").writeText("partial")
        throw IOException("disk full")
      }
      fail("should have thrown")
    } catch (e: IOException) {
      assertEquals("disk full", e.message)
    }
    assertFalse(dir.exists())
  }

  @Test
  fun cancelledCopyIsCleanedUpAndStillCancelled() {
    val dir = tempDir()
    try {
      deletingOnFailure(dir) {
        File(dir, "x").writeText("partial")
        throw CancellationException("stop")
      }
      fail("should have thrown")
    } catch (e: CancellationException) {
      // expected
    }
    assertFalse(dir.exists())
  }

  @Test
  fun successKeepsTheDirectory() {
    val dir = tempDir()
    val result = deletingOnFailure(dir) { File(dir, "ok").also { it.writeText("x") } }
    assertTrue(result.exists())
    dir.deleteRecursively()
  }
}
