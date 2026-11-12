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
