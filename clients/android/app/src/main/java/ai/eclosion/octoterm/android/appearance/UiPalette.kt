package ai.eclosion.octoterm.android.appearance

import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import kotlin.math.roundToInt

/** Same foreground/background blends and accent contrast rule as clients/web/src/appearance.ts. */
data class UiPalette(
    val light: Boolean,
    val background: Int,
    val foreground: Int,
    val panel: Int,
    val panel2: Int,
    val line: Int,
    val dim: Int,
    val accent: Int,
    val danger: Int,
)

fun TermAppearance.uiPalette(): UiPalette {
    val light = isLightTheme()
    return UiPalette(
        light = light,
        background = background,
        foreground = foreground,
        panel = mixRgb(foreground, background, if (light) 0.05 else 0.07),
        panel2 = mixRgb(foreground, background, if (light) 0.11 else 0.14),
        line = mixRgb(foreground, background, if (light) 0.2 else 0.22),
        dim = mixRgb(foreground, background, 0.55),
        accent = readableAccent(ansi[4], ansi[12], background),
        danger = readableAccent(ansi[1], ansi[9], background),
    )
}

fun TermAppearance.isLightTheme(): Boolean =
    (0.2126 * channel(background, 16) + 0.7152 * channel(background, 8) +
        0.0722 * channel(background, 0)) / 255 > 0.5

internal fun mixRgb(foreground: Int, background: Int, amount: Double): Int =
    listOf(16, 8, 0).fold(0) { rgb, shift ->
        val bg = channel(background, shift)
        rgb or ((bg + (channel(foreground, shift) - bg) * amount).roundToInt() shl shift)
    }

internal fun contrast(first: Int, second: Int): Double {
    val a = luminance(first)
    val b = luminance(second)
    return (max(a, b) + 0.05) / (min(a, b) + 0.05)
}

private fun readableAccent(normal: Int, bright: Int, background: Int): Int = when {
    contrast(normal, background) >= 3 -> normal
    contrast(bright, background) > contrast(normal, background) -> bright
    else -> normal
}

private fun luminance(rgb: Int): Double {
    fun linear(shift: Int): Double {
        val value = channel(rgb, shift) / 255.0
        return if (value <= 0.03928) value / 12.92 else ((value + 0.055) / 1.055).pow(2.4)
    }
    return 0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
}

private fun channel(rgb: Int, shift: Int): Int = (rgb ushr shift) and 0xFF
