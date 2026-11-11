package app.tovi.android

import androidx.navigation3.runtime.NavKey
import kotlinx.serialization.Serializable

/** Tabs in the bottom bar */
sealed interface Tab : NavKey

@Serializable data object Devices : Tab

@Serializable data object Transfers : Tab

@Serializable data object SettingsTab : Tab

/** Opened on top of Devices */
@Serializable data object PairDevice : NavKey
