package ai.eclosion.octoterm.android.ui.term

import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.gestures.animateScrollBy
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.FilterChip
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import ai.eclosion.octoterm.android.R
import ai.eclosion.octoterm.android.term.KeyModifiers
import ai.eclosion.octoterm.android.term.KeyStroke
import ai.eclosion.octoterm.android.term.TerminalKey
import kotlinx.coroutines.launch

@Composable
fun ExtraKeysBar(
    ctrl: Boolean,
    alt: Boolean,
    shift: Boolean,
    enabled: Boolean,
    copyEnabled: Boolean,
    scrollOffset: Int,
    onCtrl: (Boolean) -> Unit,
    onAlt: (Boolean) -> Unit,
    onShift: (Boolean) -> Unit,
    onKey: (KeyStroke) -> Unit,
    onShowKeyboard: () -> Unit,
    onCopy: () -> Unit,
    onPaste: () -> Unit,
    onJumpToBottom: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scroll = rememberScrollState()
    val scope = rememberCoroutineScope()
    var pageWidth by remember { mutableIntStateOf(0) }
    val previous = stringResource(R.string.term_keys_previous)
    val next = stringResource(R.string.term_keys_next)
    Row(modifier) {
        IconButton(
            onClick = { scope.launch { scroll.animateScrollBy(-pageWidth * 0.8f) } },
            enabled = scroll.canScrollBackward,
            modifier = Modifier.focusProperties { canFocus = false }.semantics { contentDescription = previous },
        ) { Text("‹") }
        Row(
            Modifier.weight(1f)
                .onSizeChanged { pageWidth = it.width }
                .horizontalScroll(scroll)
                .padding(horizontal = 4.dp),
        ) {
            KeyChip(stringResource(R.string.term_keyboard)) { onShowKeyboard() }
            KeyChip("Esc", enabled) { onKey(KeyStroke(TerminalKey.Escape)) }
            KeyChip("Tab", enabled) { onKey(KeyStroke(TerminalKey.Tab)) }
            listOf(Triple("Ctrl", ctrl, onCtrl), Triple("Alt", alt, onAlt), Triple("Shift", shift, onShift))
                .forEach { (label, selected, toggle) ->
                    FilterChip(
                        selected = selected, enabled = enabled,
                        onClick = { toggle(!selected) }, label = { Text(label) },
                        modifier = Modifier.padding(horizontal = 2.dp).focusProperties { canFocus = false },
                    )
                }
            KeyChip("Enter", enabled) { onKey(KeyStroke(TerminalKey.Enter)) }
            KeyChip("Ctrl+C", enabled) { onKey(KeyStroke(TerminalKey.C, modifiers = KeyModifiers(ctrl = true))) }
            listOf("↑" to TerminalKey.Up, "↓" to TerminalKey.Down, "←" to TerminalKey.Left, "→" to TerminalKey.Right,
                "Home" to TerminalKey.Home, "End" to TerminalKey.End,
                "PgUp" to TerminalKey.PageUp, "PgDn" to TerminalKey.PageDown,
                "⌫" to TerminalKey.Backspace, "Del" to TerminalKey.Delete, "Ins" to TerminalKey.Insert,
            ).forEach { (label, key) -> KeyChip(label, enabled) { onKey(KeyStroke(key)) } }
            KeyChip("Shift+Enter", enabled) { onKey(KeyStroke(TerminalKey.Enter, modifiers = KeyModifiers(shift = true))) }
            KeyChip("Ctrl+J", enabled) { onKey(KeyStroke(TerminalKey.J, modifiers = KeyModifiers(ctrl = true))) }
            KeyChip("Alt+↑", enabled) { onKey(KeyStroke(TerminalKey.Up, modifiers = KeyModifiers(alt = true))) }
            KeyChip(stringResource(R.string.term_paste), enabled) { onPaste() }
            if (copyEnabled) KeyChip(stringResource(R.string.term_copy)) { onCopy() }
            if (scrollOffset > 0) KeyChip(stringResource(R.string.term_jump_bottom)) { onJumpToBottom() }
            TerminalKey.entries.filter { it in TerminalKey.F1..TerminalKey.F12 }.forEach { key ->
                KeyChip(key.name, enabled) { onKey(KeyStroke(key)) }
            }
        }
        IconButton(
            onClick = { scope.launch { scroll.animateScrollBy(pageWidth * 0.8f) } },
            enabled = scroll.canScrollForward,
            modifier = Modifier.focusProperties { canFocus = false }.semantics { contentDescription = next },
        ) { Text("›") }
    }
}

@Composable
private fun KeyChip(label: String, enabled: Boolean = true, onClick: () -> Unit) {
    TextButton(onClick = onClick, enabled = enabled, modifier = Modifier.focusProperties { canFocus = false }) {
        Text(label)
    }
}
