package app.tovi.android.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PairingLinkTest {
  private val link = "tovi://pair/pWF2AWFrWCBQReZvvY9W-YY9_TRNPg2cjtVJIYYd5jdW8512c8vNfoDmFz"

  @Test
  fun acceptsALinkAndTrimsIt() {
    assertEquals(link, PairingLink.parse(link))
    assertEquals(link, PairingLink.parse("  $link\n"))
  }

  @Test
  fun rejectsAnythingElse() {
    assertNull(PairingLink.parse(null))
    assertNull(PairingLink.parse(""))
    assertNull(PairingLink.parse("tovi://pair/"))
    assertNull(PairingLink.parse("https://example.com/tovi://pair/abc"))
    assertNull(PairingLink.parse("TOVI://PAIR/abc"))
    // Not base64url: padding, query strings, spaces, other schemes' tricks
    assertNull(PairingLink.parse("tovi://pair/abc="))
    assertNull(PairingLink.parse("tovi://pair/abc?x=1"))
    assertNull(PairingLink.parse("tovi://pair/ab c"))
    assertNull(PairingLink.parse("tovi://pair/abc/def"))
  }
}
