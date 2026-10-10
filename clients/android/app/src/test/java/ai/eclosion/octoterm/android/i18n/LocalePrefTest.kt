package ai.eclosion.octoterm.android.i18n

import org.junit.Assert.assertEquals
import org.junit.Test

class LocalePrefTest {
    @Test
    fun defaultIsChinese() {
        assertEquals(LocalePref.ZhCN, LocalePref.DEFAULT)
        assertEquals(LocalePref.ZhCN, LocalePref.parse(null))
        assertEquals(LocalePref.ZhCN, LocalePref.parse("nope"))
    }

    @Test
    fun parseWireValues() {
        assertEquals(LocalePref.Auto, LocalePref.parse("auto"))
        assertEquals(LocalePref.ZhCN, LocalePref.parse("zh-CN"))
        assertEquals(LocalePref.En, LocalePref.parse("en"))
    }

    @Test
    fun resolveFollowsSystemWhenAuto() {
        assertEquals(AppLocale.ZhCN, resolveLocale(LocalePref.Auto, listOf("zh-CN")))
        assertEquals(AppLocale.ZhCN, resolveLocale(LocalePref.Auto, listOf("zh-Hans-CN", "en-US")))
        assertEquals(AppLocale.En, resolveLocale(LocalePref.Auto, listOf("en-US")))
        assertEquals(AppLocale.En, resolveLocale(LocalePref.Auto, emptyList()))
    }

    @Test
    fun pinnedPrefIgnoresSystem() {
        assertEquals(AppLocale.ZhCN, resolveLocale(LocalePref.ZhCN, listOf("en-US")))
        assertEquals(AppLocale.En, resolveLocale(LocalePref.En, listOf("zh-CN")))
    }
}
