package ai.eclosion.octoterm.android.ui.term

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import ai.eclosion.octoterm.android.R
import ai.eclosion.octoterm.android.term.VtEmulator

@Composable
fun ExtraKeysBar(
    ctrl: Boolean,
    alt: Boolean,
    emulator: VtEmulator,
    onCtrl: (Boolean) -> Unit,
    onAlt: (Boolean) -> Unit,
    onBytes: (ByteArray) -> Unit,
    onShowKeyboard: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier
            .horizontalScroll(rememberScrollState())
            .padding(horizontal = 4.dp),
    ) {
        KeyChip(stringResource(R.string.term_keyboard)) { onShowKeyboard() }
        KeyChip("Esc") { onBytes(byteArrayOf(0x1b)) }
        KeyChip("Tab") { onBytes(byteArrayOf('\t'.code.toByte())) }
        FilterChip(
            selected = ctrl,
            onClick = { onCtrl(!ctrl) },
            label = { Text("Ctrl") },
            modifier = Modifier.padding(horizontal = 2.dp),
        )
        FilterChip(
            selected = alt,
            onClick = { onAlt(!alt) },
            label = { Text("Alt") },
            modifier = Modifier.padding(horizontal = 2.dp),
        )
        KeyChip("Ctrl-C") { onBytes(byteArrayOf(0x03)) }
        KeyChip("↑") { onBytes(emulator.encodeArrow(0, -1)) }
        KeyChip("↓") { onBytes(emulator.encodeArrow(0, 1)) }
        KeyChip("←") { onBytes(emulator.encodeArrow(-1, 0)) }
        KeyChip("→") { onBytes(emulator.encodeArrow(1, 0)) }
        Text(
            text = stringResource(R.string.term_keys_hint),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 8.dp, top = 12.dp),
        )
    }
}

@Composable
private fun KeyChip(label: String, onClick: () -> Unit) {
    TextButton(onClick = onClick, modifier = Modifier.padding(horizontal = 0.dp)) {
        Text(label)
    }
}
