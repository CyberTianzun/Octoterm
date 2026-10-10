package ai.eclosion.octoterm.android.i18n

import android.content.Context
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.os.LocaleListCompat

object LocaleStore {
    fun load(context: Context): LocalePref {
        val raw = context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getString(KEY, LocalePref.DEFAULT.wire)
        return LocalePref.parse(raw)
    }

    fun save(context: Context, pref: LocalePref) {
        context.applicationContext
            .getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .edit()
            .putString(KEY, pref.wire)
            .apply()
    }

    fun apply(pref: LocalePref) {
        val locales = when (pref) {
            LocalePref.Auto -> LocaleListCompat.getEmptyLocaleList()
            LocalePref.ZhCN -> LocaleListCompat.forLanguageTags(AppLocale.ZhCN.tag)
            LocalePref.En -> LocaleListCompat.forLanguageTags(AppLocale.En.tag)
        }
        AppCompatDelegate.setApplicationLocales(locales)
    }

    private const val PREFS = "octoterm.locale"
    private const val KEY = "pref"
}
