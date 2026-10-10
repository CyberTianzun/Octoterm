package ai.eclosion.octoterm.android.ui.term

import android.graphics.Typeface
import android.os.SystemClock
import android.view.inputmethod.InputMethodManager
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import ai.eclosion.octoterm.android.R
import ai.eclosion.octoterm.android.appearance.TermAppearance
import ai.eclosion.octoterm.android.appearance.TermFontKind
import ai.eclosion.octoterm.android.term.TermFrame
import ai.eclosion.octoterm.android.term.TermGeometry
import ai.eclosion.octoterm.android.term.TermSelection
import ai.eclosion.octoterm.android.term.TerminalView
import ai.eclosion.octoterm.android.term.TerminalKeyboard
import ai.eclosion.octoterm.android.term.KeyModifiers
import ai.eclosion.octoterm.android.term.VtEmulator
import ai.eclosion.octoterm.android.wire.SessionInfo

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TerminalScreen(
    title: String,
    tabs: List<SessionInfo>,
    activeId: Long,
    reconnecting: Boolean,
    serverOs: String,
    emulator: VtEmulator,
    frame: TermFrame,
    generation: Long,
    appearance: TermAppearance,
    selection: TermSelection?,
    scrollOffset: Int,
    mouseTracking: Boolean,
    onBack: () -> Unit,
    onFocus: (Long) -> Unit,
    onCloseTab: (Long) -> Unit,
    onInput: (ByteArray) -> Unit,
    onProposeSize: (Int, Int) -> Unit,
    onScroll: (Int) -> Unit,
    onSelection: (TermSelection?) -> Unit,
    onMouse: (col: Int, row: Int, pressed: Boolean, moving: Boolean) -> Unit,
    onCopy: () -> Unit,
    onPaste: () -> Unit,
    onJumpToBottom: () -> Unit,
) {
    var ctrl by remember(activeId) { mutableStateOf(false) }
    var alt by remember(activeId) { mutableStateOf(false) }
    var shift by remember(activeId) { mutableStateOf(false) }
    val keyboard = remember(activeId, serverOs) { TerminalKeyboard(serverOs) }
    var termView by remember { mutableStateOf<TerminalView?>(null) }
    var viewport by remember { mutableStateOf(IntSize.Zero) }
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val terminalHint = stringResource(R.string.term_keys_hint)
    val density = LocalDensity.current
    val touchSlop = LocalViewConfiguration.current.touchSlop
    val textSizePx = with(density) { appearance.fontSizeSp.sp.toPx() }
    val typeface = remember(appearance.font) {
        when (appearance.font) {
            TermFontKind.Mono -> Typeface.MONOSPACE
            TermFontKind.Serif -> Typeface.SERIF
            TermFontKind.Sans -> Typeface.SANS_SERIF
        }
    }
    val paint = remember(textSizePx, typeface) { measureTerm(textSizePx, typeface) }
    fun clearModifiers() {
        ctrl = false; alt = false; shift = false
        termView?.softModifiers = KeyModifiers()
    }
    fun releaseKeyboard() { termView?.releaseKeys(); clearModifiers() }
    DisposableEffect(keyboard, lifecycle) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) {
                onInput(keyboard.releaseAll())
                clearModifiers()
            }
        }
        lifecycle.addObserver(observer)
        onDispose {
            lifecycle.removeObserver(observer)
            // This callback captures the old session, so releases never go to the new tab.
            onInput(keyboard.releaseAll())
        }
    }
    LaunchedEffect(keyboard, reconnecting) {
        if (reconnecting) { keyboard.reset(); clearModifiers() }
    }
    BackHandler { releaseKeyboard(); onBack() }
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title) },
                navigationIcon = {
                    IconButton(onClick = { releaseKeyboard(); onBack() }) {
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
                shift = shift,
                enabled = !reconnecting,
                copyEnabled = selection != null,
                scrollOffset = scrollOffset,
                onCtrl = {
                    ctrl = it
                    termView?.softModifiers = KeyModifiers(shift = shift, ctrl = ctrl, alt = alt)
                },
                onAlt = {
                    alt = it
                    termView?.softModifiers = KeyModifiers(shift = shift, ctrl = ctrl, alt = alt)
                },
                onShift = {
                    shift = it
                    termView?.softModifiers = KeyModifiers(shift = shift, ctrl = ctrl, alt = alt)
                },
                onKey = { stroke ->
                    if (!reconnecting) {
                        val modifiers = stroke.modifiers + KeyModifiers(shift = shift, ctrl = ctrl, alt = alt)
                        onInput(keyboard.tap(stroke.copy(modifiers = modifiers), emulator.applicationCursor))
                        clearModifiers()
                    }
                },
                onShowKeyboard = { termView?.showKeyboard() },
                onCopy = onCopy,
                onPaste = onPaste,
                onJumpToBottom = onJumpToBottom,
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
            if (tabs.size > 1) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .horizontalScroll(rememberScrollState())
                        .padding(horizontal = 8.dp),
                ) {
                    tabs.forEach { session ->
                        FilterChip(
                            selected = session.id == activeId,
                            onClick = { releaseKeyboard(); onFocus(session.id) },
                            label = { Text(session.name.ifBlank { "#${session.id}" }) },
                            trailingIcon = {
                                TextButton(onClick = {
                                    if (session.id == activeId) releaseKeyboard()
                                    onCloseTab(session.id)
                                }) {
                                    Text(stringResource(R.string.term_tab_close))
                                }
                            },
                            modifier = Modifier.padding(end = 4.dp),
                        )
                    }
                }
            }
            if (reconnecting) {
                LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            Box(
                Modifier
                    .fillMaxWidth()
                    .weight(1f)
                    .onSizeChanged { viewport = it },
            ) {
                val committed = remember(generation) { frame }
                TerminalCanvas(
                    frame = committed,
                    appearance = appearance,
                    selection = selection,
                    paint = paint,
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
                        if (view.keyboard !== keyboard) view.releaseKeys()
                        view.keyboard = keyboard
                        view.inputEnabled = !reconnecting
                        view.emulator = emulator
                        view.contentDescription = terminalHint
                        view.softModifiers = KeyModifiers(shift = shift, ctrl = ctrl, alt = alt)
                        view.onModifiersConsumed = ::clearModifiers
                        view.onInput = onInput
                    },
                    modifier = Modifier.fillMaxSize(),
                )
                Box(
                    Modifier
                        .fillMaxSize()
                        .pointerInput(mouseTracking, paint, frame.cols, frame.rows, viewport) {
                            val cols = frame.cols
                            val rows = frame.rows
                            val (ox, oy) = TermGeometry.origin(
                                viewport.width.toFloat(),
                                viewport.height.toFloat(),
                                cols,
                                rows,
                                paint.cellW,
                                paint.cellH,
                            )
                            fun cellOf(x: Float, y: Float) = TermGeometry.cellAt(
                                x, y, ox, oy, paint.cellW, paint.cellH, cols, rows,
                            )
                            awaitEachGesture {
                                val down = awaitFirstDown(requireUnconsumed = false)
                                val pointerId = down.id
                                val start = down.position
                                var pastSlop = false
                                var selecting = false
                                var twoFinger = false
                                var accum = 0f
                                var mouseDown = false
                                val anchor = cellOf(start.x, start.y)
                                val longPressAt = SystemClock.uptimeMillis() + 380L
                                if (mouseTracking && anchor != null) {
                                    mouseDown = true
                                    onMouse(anchor.first, anchor.second, true, false)
                                }
                                while (true) {
                                    val event = awaitPointerEvent()
                                    val pressed = event.changes.filter { it.pressed }
                                    if (pressed.size >= 2) {
                                        twoFinger = true
                                        if (mouseDown && anchor != null) {
                                            onMouse(anchor.first, anchor.second, false, false)
                                            mouseDown = false
                                        }
                                        val dy = pressed[0].positionChange().y
                                        val (next, lines) = TermGeometry.scrollDelta(accum, dy, paint.cellH)
                                        accum = next
                                        if (lines != 0) onScroll(lines)
                                        event.changes.forEach { it.consume() }
                                        continue
                                    }
                                    val change = event.changes.firstOrNull { it.id == pointerId }
                                    if (change == null || !change.pressed) {
                                        val end = change?.position ?: start
                                        val cell = cellOf(end.x, end.y) ?: anchor
                                        when {
                                            mouseDown && cell != null -> onMouse(cell.first, cell.second, false, false)
                                            selecting && anchor != null && cell != null ->
                                                onSelection(TermSelection(anchor.first, anchor.second, cell.first, cell.second))
                                            !pastSlop && !twoFinger -> {
                                                onSelection(null)
                                                termView?.showKeyboard()
                                            }
                                        }
                                        break
                                    }
                                    val dist = (change.position - start).getDistance()
                                    if (!pastSlop && dist > touchSlop) pastSlop = true
                                    val cell = cellOf(change.position.x, change.position.y)
                                    if (!selecting && !pastSlop &&
                                        SystemClock.uptimeMillis() >= longPressAt && anchor != null
                                    ) {
                                        selecting = true
                                        if (mouseDown) {
                                            onMouse(anchor.first, anchor.second, false, false)
                                            mouseDown = false
                                        }
                                        onSelection(TermSelection(anchor.first, anchor.second, anchor.first, anchor.second))
                                    }
                                    when {
                                        selecting && anchor != null && cell != null ->
                                            onSelection(TermSelection(anchor.first, anchor.second, cell.first, cell.second))
                                        mouseTracking && mouseDown && cell != null && pastSlop ->
                                            onMouse(cell.first, cell.second, true, true)
                                        !mouseTracking && !selecting && pastSlop -> {
                                            val (next, lines) = TermGeometry.scrollDelta(
                                                accum,
                                                change.positionChange().y,
                                                paint.cellH,
                                            )
                                            accum = next
                                            if (lines != 0) onScroll(lines)
                                        }
                                    }
                                    change.consume()
                                }
                            }
                        },
                )
            }
            DisposableEffect(Unit) {
                val imm = context.getSystemService(InputMethodManager::class.java)
                onDispose {
                    termView?.windowToken?.let { imm?.hideSoftInputFromWindow(it, 0) }
                }
            }
        }
    }
}
