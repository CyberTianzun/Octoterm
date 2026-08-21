package ai.eclosion.octoterm.android.ui.term

import android.view.inputmethod.InputMethodManager
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.viewinterop.AndroidView
import ai.eclosion.octoterm.android.R
import ai.eclosion.octoterm.android.term.TermFrame
import ai.eclosion.octoterm.android.term.TerminalView
import ai.eclosion.octoterm.android.term.VtEmulator

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TerminalScreen(
    title: String,
    reconnecting: Boolean,
    emulator: VtEmulator,
    frame: TermFrame,
    generation: Long,
    onBack: () -> Unit,
    onInput: (ByteArray) -> Unit,
    onProposeSize: (Int, Int) -> Unit,
) {
    var ctrl by remember { mutableStateOf(false) }
    var alt by remember { mutableStateOf(false) }
    var termView by remember { mutableStateOf<TerminalView?>(null) }
    val context = LocalContext.current
    BackHandler(onBack = onBack)
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title) },
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
        bottomBar = {
            ExtraKeysBar(
                ctrl = ctrl,
                alt = alt,
                emulator = emulator,
                onCtrl = { ctrl = it },
                onAlt = { alt = it },
                onBytes = { raw -> onInput(applyModifiers(raw, ctrl, alt).also { ctrl = false; alt = false }) },
                onShowKeyboard = { termView?.showKeyboard() },
                modifier = Modifier
                    .fillMaxWidth()
                    .navigationBarsPadding(),
            )
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            if (reconnecting) {
                LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            Box(
                Modifier
                    .fillMaxWidth()
                    .weight(1f),
            ) {
                val committed = remember(generation) { frame }
                TerminalCanvas(
                    frame = committed,
                    onProposeSize = onProposeSize,
                    modifier = Modifier.fillMaxSize(),
                )
                AndroidView(
                    factory = { ctx ->
                        TerminalView(ctx).also { view ->
                            view.emulator = emulator
                            termView = view
                        }
                    },
                    update = { view ->
                        view.emulator = emulator
                        view.onInput = { raw ->
                            onInput(applyModifiers(raw, ctrl, alt))
                            ctrl = false
                            alt = false
                        }
                    },
                    modifier = Modifier.fillMaxSize(),
                )
            }
            DisposableEffect(Unit) {
                val imm = context.getSystemService(InputMethodManager::class.java)
                onDispose { imm?.hideSoftInputFromWindow(null, 0) }
            }
        }
    }
}

private fun applyModifiers(raw: ByteArray, ctrl: Boolean, alt: Boolean): ByteArray {
    var out = raw
    if (ctrl && out.size == 1) {
        val c = out[0].toInt() and 0xff
        if (c in 64..127) out = byteArrayOf((c and 0x1f).toByte())
    }
    if (alt) out = byteArrayOf(0x1b) + out
    return out
}
