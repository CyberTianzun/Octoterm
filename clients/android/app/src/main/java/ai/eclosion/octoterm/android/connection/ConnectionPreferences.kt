package ai.eclosion.octoterm.android.connection

import android.content.Context

class ConnectionPreferences(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences("octoterm.connection-runtime", Context.MODE_PRIVATE)
    var keepInBackground: Boolean
        get() = prefs.getBoolean("keep-background", true)
        set(value) { prefs.edit().putBoolean("keep-background", value).apply() }
    var desiredConnections: Set<String>
        get() = prefs.getStringSet("desired", emptySet()).orEmpty().toSet()
        set(value) { prefs.edit().putStringSet("desired", value.toSet()).apply() }
}
