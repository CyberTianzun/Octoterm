package ai.eclosion.octoterm.android.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import ai.eclosion.octoterm.android.appearance.AppearanceCodec
import ai.eclosion.octoterm.android.appearance.TermAppearance
import ai.eclosion.octoterm.android.appearance.contrast
import ai.eclosion.octoterm.android.appearance.mixRgb
import ai.eclosion.octoterm.android.appearance.uiPalette

private fun rgb(value: Int): Color = Color(value or 0xFF000000.toInt())

private fun onColor(background: Int): Color =
    if (contrast(0xFFFFFF, background) >= contrast(0x000000, background)) Color.White else Color.Black

@Composable
fun OctotermTheme(
    appearance: TermAppearance = AppearanceCodec.default(isSystemInDarkTheme()),
    content: @Composable () -> Unit,
) {
    val colors = remember(appearance) {
        val p = appearance.uiPalette()
        val base = if (p.light) lightColorScheme() else darkColorScheme()
        base.copy(
            primary = rgb(p.accent),
            onPrimary = onColor(p.accent),
            primaryContainer = rgb(p.panel2),
            onPrimaryContainer = rgb(p.accent),
            secondary = rgb(p.accent),
            onSecondary = onColor(p.accent),
            secondaryContainer = rgb(p.panel2),
            onSecondaryContainer = rgb(p.foreground),
            tertiary = rgb(p.accent),
            onTertiary = onColor(p.accent),
            tertiaryContainer = rgb(p.panel2),
            onTertiaryContainer = rgb(p.foreground),
            background = rgb(p.background),
            onBackground = rgb(p.foreground),
            surface = rgb(p.panel),
            onSurface = rgb(p.foreground),
            surfaceVariant = rgb(p.panel2),
            onSurfaceVariant = rgb(p.dim),
            surfaceTint = Color.Transparent,
            surfaceBright = rgb(if (p.light) p.background else p.panel2),
            surfaceDim = rgb(if (p.light) p.panel2 else p.background),
            surfaceContainerLowest = rgb(p.background),
            surfaceContainerLow = rgb(p.panel),
            surfaceContainer = rgb(p.panel),
            surfaceContainerHigh = rgb(p.panel2),
            surfaceContainerHighest = rgb(p.panel2),
            outline = rgb(p.dim),
            outlineVariant = rgb(p.line),
            inverseSurface = rgb(p.foreground),
            inverseOnSurface = rgb(p.background),
            inversePrimary = rgb(p.background),
            error = rgb(p.danger),
            onError = onColor(p.danger),
            errorContainer = rgb(mixRgb(p.danger, p.background, 0.14)),
            onErrorContainer = rgb(p.foreground),
            scrim = Color.Black,
        )
    }
    MaterialTheme(colorScheme = colors, content = content)
}
