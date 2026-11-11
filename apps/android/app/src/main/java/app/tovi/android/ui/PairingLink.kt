package app.tovi.android.ui

/** `tovi://pair/<base64url>` links, as shown in the computer's QR code */
object PairingLink {
  const val PREFIX = "tovi://pair/"

  /**
   * The pairing link in [raw] (scanned text, pasted text, or an opened URI),
   * or null if it isn't one. Only a quick shape check: the core decodes and
   * verifies it.
   */
  fun parse(raw: String?): String? {
    val link = raw?.trim() ?: return null
    if (!link.startsWith(PREFIX)) return null
    val payload = link.substring(PREFIX.length)
    return link.takeIf { payload.isNotEmpty() && payload.all(::isBase64Url) }
  }

  private fun isBase64Url(c: Char) = c in 'A'..'Z' || c in 'a'..'z' || c in '0'..'9' || c == '-' || c == '_'
}
