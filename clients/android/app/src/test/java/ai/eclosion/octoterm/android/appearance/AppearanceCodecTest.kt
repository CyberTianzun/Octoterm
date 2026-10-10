package ai.eclosion.octoterm.android.appearance

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class AppearanceCodecTest {
    @Test
    fun defaultsSelect2026ThemeForSystemMode() {
        assertEquals(AppearanceCodec.builtin("2026 Light"), AppearanceCodec.default(prefersDark = false))
        assertEquals(AppearanceCodec.builtin("2026 Dark"), AppearanceCodec.default(prefersDark = true))
    }

    @Test
    fun systemThemeChangesKeepFontAndScrollback() {
        val customized = AppearanceCodec.default(false)
            .withFontSize(18f).withFont(TermFontKind.Serif).withScrollback(5000)
        val dark = customized.withTheme(AppearanceCodec.default(true))
        assertEquals("2026 Dark", dark.name)
        assertEquals(18f, dark.fontSizeSp)
        assertEquals(TermFontKind.Serif, dark.font)
        assertEquals(5000, dark.scrollback)
        assertEquals(customized, dark.withTheme(AppearanceCodec.default(false)))
    }

    @Test
    fun roundTripKeepsColorsSizeAndScrollback() {
        val theme = AppearanceCodec.builtin("2026 Dark")!!
        val edited = theme.withFontSize(18f).withScrollback(5000).withFont(TermFontKind.Mono)
        val back = AppearanceCodec.importJson(AppearanceCodec.exportJson(edited))
        assertEquals(edited.name, back!!.name)
        assertEquals(edited.foreground, back.foreground)
        assertEquals(edited.background, back.background)
        assertEquals(edited.ansi, back.ansi)
        assertEquals(18f, back.fontSizeSp)
        assertEquals(5000, back.scrollback)
    }

    @Test
    fun badFieldsFallBackAndGarbageIsRejected() {
        assertNull(AppearanceCodec.importJson("not json"))
        val base = AppearanceCodec.default().withFontSize(16f)
        val imported = AppearanceCodec.importJson(
            """{"theme":{"name":"2026 Light","colors":{"background":"red","foreground":"#112233"}},"font":{"size":99},"terminal":{"scrollback":-4}}""",
            base,
        )!!
        assertEquals("2026 Light", imported.name)
        assertEquals(0x112233, imported.foreground)
        assertEquals(base.background, imported.background)
        assertEquals(32f, imported.fontSizeSp)
        assertEquals(0, imported.scrollback)
    }

    @Test
    fun themeNameAloneSelectsBuiltin() {
        val imported = AppearanceCodec.importJson("""{"theme":{"name":"Tokyo Night"}}""")!!
        assertEquals(AppearanceCodec.builtin("Tokyo Night")!!.background, imported.background)
    }
}
