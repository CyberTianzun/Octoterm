package ai.eclosion.octoterm.android.appearance

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class UiPaletteTest {
    @Test
    fun dark2026MatchesWebUiColors() {
        val palette = AppearanceCodec.default(true).uiPalette()
        assertFalse(palette.light)
        assertEquals(0x191A1B, palette.background)
        assertEquals(0xCCCCCC, palette.foreground)
        assertEquals(0x262627, palette.panel)
        assertEquals(0x323334, palette.panel2)
        assertEquals(0x404142, palette.line)
        assertEquals(0x7B7C7C, palette.dim)
        assertEquals(0x2472C8, palette.accent)
        assertEquals(0xCD3131, palette.danger)
    }

    @Test
    fun light2026MatchesWebUiColors() {
        val palette = AppearanceCodec.default(false).uiPalette()
        assertTrue(palette.light)
        assertEquals(0xFAFAFD, palette.background)
        assertEquals(0x3B3B3B, palette.foreground)
        assertEquals(0xF0F0F3, palette.panel)
        assertEquals(0xE5E5E8, palette.panel2)
        assertEquals(0xD4D4D6, palette.line)
        assertEquals(0x919192, palette.dim)
        assertEquals(0x0451A5, palette.accent)
        assertEquals(0xCD3131, palette.danger)
    }

    @Test
    fun lowContrastAccentUsesBrighterVariant() {
        val theme = AppearanceCodec.default(true)
        val ansi = theme.ansi.toMutableList().apply {
            this[4] = theme.background
            this[12] = 0xFFFFFF
        }
        assertEquals(0xFFFFFF, theme.copy(ansi = ansi).uiPalette().accent)
    }
}
