package app.tovi.android.share

private const val ACTION_SEND = "android.intent.action.SEND"
private const val ACTION_SEND_MULTIPLE = "android.intent.action.SEND_MULTIPLE"

/** The streams a share intent carries; a text-only share has none */
fun <T> sharedStreams(action: String?, single: T?, multiple: List<T?>?): List<T> =
  when (action) {
    ACTION_SEND -> listOfNotNull(single)
    ACTION_SEND_MULTIPLE -> multiple.orEmpty().filterNotNull()
    else -> emptyList()
  }

/** At most this many files per share; the grant list rides in one binder transaction */
const val MAX_SHARED_FILES = 200

/**
 * Only `content://` documents are readable on the sender's behalf. A `file://`
 * URI would be opened with TOVI's own permissions, including its private data.
 */
fun isShareableUri(scheme: String?): Boolean = scheme.equals("content", ignoreCase = true)
