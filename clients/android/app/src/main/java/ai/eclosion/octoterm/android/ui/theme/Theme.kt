package ai.eclosion.octoterm.android.ui.theme

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val Background = Color(0xFF1A1B26)
private val Surface = Color(0xFF24283B)
private val OnSurface = Color(0xFFC0CAF5)
private val Accent = Color(0xFF7AA2F7)

private val DarkColors = darkColorScheme(
    primary = Accent,
    onPrimary = Color(0xFF1A1B26),
    primaryContainer = Color(0xFF3D59A1),
    onPrimaryContainer = Color(0xFFC0CAF5),
    background = Background,
    onBackground = OnSurface,
    surface = Surface,
    onSurface = OnSurface,
    onSurfaceVariant = Color(0xFF9AA5CE),
    surfaceContainer = Color(0xFF1F2335),
    surfaceContainerHigh = Color(0xFF292E42),
    error = Color(0xFFF7768E),
)

@Composable
fun OctotermTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = DarkColors,
        content = content,
    )
}
