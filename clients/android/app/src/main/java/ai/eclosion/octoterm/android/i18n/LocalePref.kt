package ai.eclosion.octoterm.android.i18n

/**
 * 用户语言偏好。和 web 的 LocalePref 对齐：auto 跟随系统，其余钉死一种语言。
 * 默认是中文，所以英文系统上第一次打开也是中文界面。
 */
enum class LocalePref(val wire: String) {
    Auto("auto"),
    ZhCN("zh-CN"),
    En("en"),
    ;

    companion object {
        val DEFAULT: LocalePref = ZhCN

        fun parse(raw: String?): LocalePref {
            return entries.firstOrNull { it.wire == raw } ?: DEFAULT
        }
    }
}

enum class AppLocale(val tag: String) {
    ZhCN("zh-CN"),
    En("en"),
}

fun resolveLocale(pref: LocalePref, systemTags: List<String>): AppLocale {
    return when (pref) {
        LocalePref.ZhCN -> AppLocale.ZhCN
        LocalePref.En -> AppLocale.En
        LocalePref.Auto -> {
            val hit = systemTags.any { tag ->
                val lower = tag.lowercase()
                lower == "zh" || lower.startsWith("zh-")
            }
            if (hit) AppLocale.ZhCN else AppLocale.En
        }
    }
}
