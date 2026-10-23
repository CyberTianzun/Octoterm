package ai.eclosion.octoterm.android.ui.appearance

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.FlowRow
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
import androidx.compose.material3.Switch
import androidx.compose.foundation.layout.Row
import androidx.compose.ui.Alignment
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import ai.eclosion.octoterm.android.R
import ai.eclosion.octoterm.android.ui.components.AppBarTitle
import ai.eclosion.octoterm.android.appearance.AppearanceCodec
import ai.eclosion.octoterm.android.appearance.TermAppearance
import ai.eclosion.octoterm.android.appearance.TermFontKind
import ai.eclosion.octoterm.android.i18n.LocalePref
import ai.eclosion.octoterm.android.term.AgentKeyLayout

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
    localePref: LocalePref,
    onLocalePref: (LocalePref) -> Unit,
    extraKeyRows: Int,
    onExtraKeyRows: (Int) -> Unit,
    agentKeyLayouts: Map<String, AgentKeyLayout>,
    onAgentKeyLayout: (String, AgentKeyLayout) -> Unit,
    keepConnectionsInBackground: Boolean,
    onKeepConnectionsInBackground: (Boolean) -> Unit,
) {
    var scrollDraft by remember(appearance.scrollback) { mutableStateOf(appearance.scrollback.toString()) }
    val systemThemeName = AppearanceCodec.default(isSystemInDarkTheme()).name
    Scaffold(
        topBar = {
            TopAppBar(
                title = { AppBarTitle(stringResource(R.string.settings_title)) },
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
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Column(Modifier.weight(1f).padding(end = 12.dp)) {
                    Text(stringResource(R.string.connections_background_title), style = MaterialTheme.typography.titleSmall)
                    Text(stringResource(R.string.connections_background_hint), style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                Switch(checked = keepConnectionsInBackground, onCheckedChange = onKeepConnectionsInBackground)
            }
            Text(stringResource(R.string.locale_action), style = MaterialTheme.typography.titleMedium)
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                LocalePref.entries.forEach { pref ->
                    FilterChip(
                        selected = localePref == pref,
                        onClick = { onLocalePref(pref) },
                        label = { Text(stringResource(when (pref) {
                            LocalePref.Auto -> R.string.locale_auto
                            LocalePref.ZhCN -> R.string.locale_zh
                            LocalePref.En -> R.string.locale_en
                        })) },
                    )
                }
            }
            Text(stringResource(R.string.settings_keyboard_rows), style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 12.dp))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (rows in 1..2) {
                    FilterChip(
                        selected = extraKeyRows == rows,
                        onClick = { onExtraKeyRows(rows) },
                        label = { Text(stringResource(if (rows == 1) R.string.settings_keyboard_one_row else R.string.settings_keyboard_two_rows)) },
                    )
                }
            }
            AgentKeyboardSettings(agentKeyLayouts, onAgentKeyLayout)
            Text(stringResource(R.string.appearance_theme), style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 16.dp))
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
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(top = 8.dp)) {
                TermFontKind.entries.forEach { kind ->
                    FilterChip(
                        selected = appearance.font == kind,
                        onClick = { onFont(kind) },
                        label = {
                            Text(
                                stringResource(
                                    when (kind) {
                                        TermFontKind.JetBrainsMono -> R.string.appearance_font_jetbrains_mono
                                        TermFontKind.GoogleSansCode -> R.string.appearance_font_google_sans_code
                                        TermFontKind.ZedMono -> R.string.appearance_font_zed_mono
                                        TermFontKind.Mono -> R.string.appearance_font_mono
                                    },
                                ),
                            )
                        },
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
