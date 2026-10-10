package ai.eclosion.octoterm.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.core.view.WindowCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import ai.eclosion.octoterm.android.appearance.isLightTheme
import ai.eclosion.octoterm.android.ui.connections.ConnectionViewModel
import ai.eclosion.octoterm.android.ui.theme.OctotermTheme

class MainActivity : ComponentActivity() {
    private val connectionViewModel: ConnectionViewModel by viewModels { ConnectionViewModel.Factory }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            val state by connectionViewModel.state.collectAsStateWithLifecycle()
            val prefersDark = isSystemInDarkTheme()
            LaunchedEffect(prefersDark) {
                connectionViewModel.onSystemThemeChanged(prefersDark)
            }
            SideEffect {
                WindowCompat.getInsetsController(window, window.decorView).apply {
                    isAppearanceLightStatusBars = state.appearance.isLightTheme()
                    isAppearanceLightNavigationBars = state.appearance.isLightTheme()
                }
            }
            OctotermTheme(appearance = state.appearance) {
                OctotermApp(connectionViewModel)
            }
        }
    }
}
