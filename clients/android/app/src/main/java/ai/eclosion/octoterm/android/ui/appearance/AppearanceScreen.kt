package ai.eclosion.octoterm.android.ui.appearance

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import ai.eclosion.octoterm.android.R
import ai.eclosion.octoterm.android.appearance.AppearanceCodec
import ai.eclosion.octoterm.android.appearance.TermAppearance
import ai.eclosion.octoterm.android.appearance.TermFontKind

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppearanceScreen(
    appearance: TermAppearance,
    followsSystem: Boolean,
    onFollowSystem: () -> Unit,
    onBack: () -> Unit,
    onFontSize: (Float) -> Unit,
    onFont: (TermFontKind) -> Unit,
    onScrollback: (Int) -> Unit,
    onTheme: (String) -> Unit,
    onCopy: () -> Unit,
    onImport: () -> Unit,
) {
    var scrollDraft by remember(appearance.scrollback) { mutableStateOf(appearance.scrollback.toString()) }
    val systemThemeName = AppearanceCodec.default(isSystemInDarkTheme()).name
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.appearance_title)) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = stringResource(R.string.action_back),
                        )
                    }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 8.dp),
        ) {
            Text(stringResource(R.string.appearance_theme), style = MaterialTheme.typography.titleMedium)
            FilterChip(
                selected = followsSystem,
                onClick = onFollowSystem,
                label = {
                    Text(stringResource(R.string.appearance_theme_system, systemThemeName))
                },
                modifier = Modifier.padding(top = 8.dp, end = 8.dp),
            )
            AppearanceCodec.builtins().forEach { theme ->
                FilterChip(
                    selected = !followsSystem && theme.name == appearance.name,
                    onClick = { onTheme(theme.name) },
                    label = { Text(theme.name) },
                    modifier = Modifier.padding(top = 8.dp, end = 8.dp),
                )
            }
            Text(
                stringResource(R.string.appearance_font_size, appearance.fontSizeSp.toInt()),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.padding(top = 20.dp),
            )
            Slider(
                value = appearance.fontSizeSp,
                onValueChange = onFontSize,
                valueRange = 8f..32f,
                modifier = Modifier.fillMaxWidth(),
            )
            Text(
                stringResource(R.string.appearance_font),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.padding(top = 8.dp),
            )
            Column(Modifier.padding(top = 8.dp)) {
                TermFontKind.entries.forEach { kind ->
                    FilterChip(
                        selected = appearance.font == kind,
                        onClick = { onFont(kind) },
                        label = {
                            Text(
                                stringResource(
                                    when (kind) {
                                        TermFontKind.Mono -> R.string.appearance_font_mono
                                        TermFontKind.Serif -> R.string.appearance_font_serif
                                        TermFontKind.Sans -> R.string.appearance_font_sans
                                    },
                                ),
                            )
                        },
                        modifier = Modifier.padding(end = 8.dp, bottom = 8.dp),
                    )
                }
            }
            Text(
                stringResource(R.string.appearance_scrollback),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.padding(top = 12.dp),
            )
            OutlinedTextField(
                value = scrollDraft,
                onValueChange = { raw ->
                    scrollDraft = raw.filter { it.isDigit() }.take(6)
                    scrollDraft.toIntOrNull()?.let(onScrollback)
                },
                singleLine = true,
                label = { Text(stringResource(R.string.appearance_scrollback_hint)) },
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 8.dp),
            )
            Text(
                stringResource(R.string.appearance_import_hint),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 20.dp),
            )
            TextButton(onClick = onCopy, modifier = Modifier.padding(top = 8.dp)) {
                Text(stringResource(R.string.appearance_export))
            }
            TextButton(onClick = onImport) {
                Text(stringResource(R.string.appearance_import))
            }
        }
    }
}
