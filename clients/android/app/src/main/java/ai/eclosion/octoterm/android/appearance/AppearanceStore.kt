package ai.eclosion.octoterm.android.appearance

import android.content.Context

class AppearanceStore(context: Context) {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun followsSystem(): Boolean = prefs.getBoolean(KEY_FOLLOW_SYSTEM, !prefs.contains(KEY))

    fun load(prefersDark: Boolean): TermAppearance {
        val base = AppearanceCodec.default(prefersDark)
        val saved = prefs.getString(KEY, null)?.let { AppearanceCodec.importJson(it, base) } ?: base
        return if (followsSystem()) saved.withTheme(base) else saved
    }

    fun save(appearance: TermAppearance, followsSystem: Boolean) {
        prefs.edit()
            .putString(KEY, AppearanceCodec.exportJson(appearance))
            .putBoolean(KEY_FOLLOW_SYSTEM, followsSystem)
            .apply()
    }

    private companion object {
        const val PREFS = "octoterm-appearance"
        const val KEY = "json"
        const val KEY_FOLLOW_SYSTEM = "follow-system"
    }
}
