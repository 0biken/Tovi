package app.tovi.android.share

import org.junit.Test
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue

class SharedStreamsTest {
  private val send = "android.intent.action.SEND"
  private val sendMultiple = "android.intent.action.SEND_MULTIPLE"

  @Test
  fun sendWithASingleItemGivesThatItem() {
    assertEquals(listOf("a"), sharedStreams(send, "a", null))
  }

  @Test
  fun sendMultipleKeepsTheOrder() {
    assertEquals(listOf("a", "b", "c"), sharedStreams(sendMultiple, null, listOf("a", "b", "c")))
  }

  @Test
  fun aTextShareHasNoStream() {
    assertEquals(emptyList<String>(), sharedStreams<String>(send, null, null))
  }

  @Test
  fun otherActionsGiveNothing() {
    assertEquals(emptyList<String>(), sharedStreams("android.intent.action.VIEW", "a", listOf("b")))
    assertEquals(emptyList<String>(), sharedStreams<String>(null, "a", listOf("b")))
  }

  @Test
  fun nullsInsideMultipleAreDropped() {
    assertEquals(listOf("a", "c"), sharedStreams(sendMultiple, null, listOf("a", null, "c")))
  }

  @Test
  fun onlyContentUrisAreShareable() {
    assertTrue(isShareableUri("content"))
    assertTrue(isShareableUri("CONTENT"))
    assertFalse(isShareableUri("file"))
    assertFalse(isShareableUri("android.resource"))
    assertFalse(isShareableUri("http"))
    assertFalse(isShareableUri(null))
  }
}
