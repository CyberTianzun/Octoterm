package ai.eclosion.octoterm.android.ui.connections

import android.app.Application
import android.content.ClipboardManager
import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelProvider.AndroidViewModelFactory.Companion.APPLICATION_KEY
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import ai.eclosion.octoterm.android.connection.ConnectionStore
import ai.eclosion.octoterm.android.connection.ConnectionUrl
import ai.eclosion.octoterm.android.connection.ConnectionUrlException
import ai.eclosion.octoterm.android.connection.ServerConnection
import ai.eclosion.octoterm.android.net.OctoClient
import ai.eclosion.octoterm.android.term.TermFrame
import ai.eclosion.octoterm.android.term.TermPaintGate
import ai.eclosion.octoterm.android.term.VtEmulator
import ai.eclosion.octoterm.android.wire.AttachMode
import ai.eclosion.octoterm.android.wire.ClientMsg
import ai.eclosion.octoterm.android.wire.ServerMsg
import ai.eclosion.octoterm.android.wire.SessionInfo
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import kotlin.math.abs
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.util.UUID

sealed class AppScreen {
    data object List : AppScreen()
    data class Editor(val connectionId: String?) : AppScreen()
    data class Server(val connectionId: String) : AppScreen()
}

data class EditorState(
    val name: String = "",
    val url: String = "",
    val token: String = "",
    val urlError: EditorFieldError? = null,
    val tokenError: EditorFieldError? = null,
    val loopbackWarning: Boolean = false,
    val wildcardWarning: Boolean = false,
    val clipboardUrl: String? = null,
)

enum class EditorFieldError {
    EmptyUrl,
    BadUrl,
    EmptyToken,
}

sealed class UserMessage {
    data object Saved : UserMessage()
    data object Deleted : UserMessage()
    data class AuthFailed(val serverDetail: String?) : UserMessage()
    data object Unreachable : UserMessage()
    data object Timeout : UserMessage()
    data object Closed : UserMessage()
    data object Unexpected : UserMessage()
}

data class ConnectionUiState(
    val connections: List<ServerConnection> = emptyList(),
    val screen: AppScreen = AppScreen.List,
    val editor: EditorState = EditorState(),
    val connectingId: String? = null,
    val pendingDelete: ServerConnection? = null,
    val showAddSheet: Boolean = false,
    val sessions: List<SessionInfo> = emptyList(),
    val attachedId: Long? = null,
    val reconnecting: Boolean = false,
    val pendingRename: SessionInfo? = null,
    val renameDraft: String = "",
    val termGeneration: Long = 0,
)

class ConnectionViewModel(
    private val app: Application,
) : ViewModel() {
    private val store = ConnectionStore(app)
    private val main = Handler(Looper.getMainLooper())
    private var client: OctoClient? = null
    val emulator = VtEmulator()
    @Volatile
    var painted: TermFrame = TermFrame.Empty
        private set
    private val paintGate = TermPaintGate()
    private var proposedCols = 80
    private var proposedRows = 24
    private var lastSentCols = 0
    private var lastSentRows = 0
    private var pendingAttachId: Long? = null
    private var resizeQuietUntil = 0L
    private val proposeFlush = Runnable { applyProposedSize() }
    private val publishSoon = Runnable { publishTerm() }
    private val _state = MutableStateFlow(ConnectionUiState(connections = sorted(store.load())))
    val state: StateFlow<ConnectionUiState> = _state.asStateFlow()

    private val _messages = MutableSharedFlow<UserMessage>(extraBufferCapacity = 4)
    val messages: SharedFlow<UserMessage> = _messages.asSharedFlow()

    fun openAddSheet() {
        val clip = clipboardUrl()
        if (clip == null) {
            startEditor(null, null)
        } else {
            _state.update { it.copy(showAddSheet = true, editor = it.editor.copy(clipboardUrl = clip)) }
        }
    }

    fun dismissAddSheet() {
        _state.update { it.copy(showAddSheet = false) }
    }

    fun addFromClipboard() {
        val clip = _state.value.editor.clipboardUrl ?: clipboardUrl()
        _state.update { it.copy(showAddSheet = false) }
        startEditor(null, clip)
    }

    fun addManually() {
        _state.update { it.copy(showAddSheet = false) }
        startEditor(null, null)
    }

    fun edit(id: String) {
        startEditor(id, null)
    }

    fun closeEditor() {
        _state.update {
            it.copy(screen = AppScreen.List, editor = EditorState(clipboardUrl = clipboardUrl()))
        }
    }

    fun onNameChange(value: String) {
        _state.update { it.copy(editor = it.editor.copy(name = value)) }
    }

    fun onUrlChange(value: String) {
        _state.update { it.copy(editor = refreshWarnings(it.editor.copy(url = value, urlError = null))) }
    }

    fun onTokenChange(value: String) {
        _state.update { it.copy(editor = it.editor.copy(token = value, tokenError = null)) }
    }

    fun pasteClipboardIntoEditor() {
        val clip = clipboardUrl() ?: return
        applyPrefill(_state.value.editor.copy(clipboardUrl = clip), clip)
    }

    fun saveEditor() {
        val current = _state.value
        val editor = current.editor
        val parsed = try {
            ConnectionUrl.parse(editor.url, editor.token)
        } catch (_: ConnectionUrlException) {
            _state.update {
                it.copy(editor = it.editor.copy(urlError = if (editor.url.isBlank()) EditorFieldError.EmptyUrl else EditorFieldError.BadUrl))
            }
            return
        }
        val token = parsed.token
        if (token.isNullOrEmpty()) {
            _state.update { it.copy(editor = it.editor.copy(tokenError = EditorFieldError.EmptyToken)) }
            return
        }
        val now = System.currentTimeMillis()
        val editingId = (current.screen as? AppScreen.Editor)?.connectionId
        val updated = if (editingId == null) {
            current.connections + ServerConnection(
                id = UUID.randomUUID().toString(),
                name = editor.name.trim(),
                url = editor.url.trim(),
                token = token,
                createdAt = now,
            )
        } else {
            current.connections.map { item ->
                if (item.id != editingId) item
                else item.copy(name = editor.name.trim(), url = editor.url.trim(), token = token)
            }
        }
        persist(updated)
        _state.update {
            it.copy(
                connections = sorted(updated),
                screen = AppScreen.List,
                editor = EditorState(clipboardUrl = clipboardUrl()),
            )
        }
        _messages.tryEmit(UserMessage.Saved)
    }

    fun requestDelete(connection: ServerConnection) {
        _state.update { it.copy(pendingDelete = connection) }
    }

    fun dismissDelete() {
        _state.update { it.copy(pendingDelete = null) }
    }

    fun confirmDelete() {
        val target = _state.value.pendingDelete ?: return
        val updated = _state.value.connections.filterNot { it.id == target.id }
        persist(updated)
        _state.update { it.copy(connections = updated, pendingDelete = null) }
        _messages.tryEmit(UserMessage.Deleted)
    }

    fun connect(id: String) {
        val connection = _state.value.connections.firstOrNull { it.id == id } ?: return
        if (_state.value.connectingId != null) return
        val parsed = try {
            ConnectionUrl.parse(connection.url, connection.token)
        } catch (_: ConnectionUrlException) {
            _messages.tryEmit(UserMessage.Unexpected)
            return
        }
        val token = parsed.token
        if (token.isNullOrEmpty()) {
            _messages.tryEmit(UserMessage.AuthFailed(null))
            return
        }
        _state.update { it.copy(connectingId = id) }
        val next = OctoClient()
        client?.close()
        client = next
        next.onOpen = {
            onMain {
                val now = System.currentTimeMillis()
                val updated = _state.value.connections.map { item ->
                    if (item.id == id) item.copy(lastUsedAt = now) else item
                }
                persist(updated)
                _state.update {
                    it.copy(
                        connections = sorted(updated),
                        connectingId = null,
                        reconnecting = false,
                        screen = AppScreen.Server(id),
                    )
                }
                next.send(ClientMsg.ListSessions)
            }
        }
        next.onReconnecting = { onMain { _state.update { it.copy(reconnecting = true) } } }
        next.onFatal = { message ->
            onMain {
                _state.update { it.copy(connectingId = null, reconnecting = false) }
                dropClient()
                _messages.tryEmit(UserMessage.AuthFailed(message))
                if (_state.value.screen is AppScreen.Server) {
                    _state.update {
                        it.copy(screen = AppScreen.List, sessions = emptyList(), attachedId = null)
                    }
                }
            }
        }
        next.onControl = { msg -> onMain { handleServer(msg) } }
        next.onChannelData = { channel, payload ->
            // 必须和 control 同一条主线程队列、按到达顺序处理。
            // 若在 OkHttp 线程 write、主线程 resync-begin/reset，重绘字节会被后到的 reset 清掉。
            onMain {
                if (channel == TERM_CHANNEL) {
                    emulator.write(payload)
                    next.noteData(channel, payload.size)
                    if (paintGate.shouldPublishWrite()) schedulePublish()
                }
            }
        }
        next.connect(parsed.webSocketUrl, token)
        viewModelScope.launch {
            // Handshake timeout: if still connecting, fail.
            kotlinx.coroutines.delay(12_000)
            if (_state.value.connectingId == id && _state.value.screen !is AppScreen.Server) {
                dropClient()
                _state.update { it.copy(connectingId = null) }
                _messages.tryEmit(UserMessage.Timeout)
            }
        }
    }

    fun disconnect() {
        closeTerminal()
        dropClient()
        _state.update {
            it.copy(
                screen = AppScreen.List,
                sessions = emptyList(),
                attachedId = null,
                reconnecting = false,
                pendingRename = null,
            )
        }
    }

    fun newSession() {
        client?.send(ClientMsg.NewSession())
    }

    fun openSession(sessionId: Long) {
        if (_state.value.attachedId != null) {
            client?.detach(TERM_CHANNEL)
        }
        main.removeCallbacks(proposeFlush)
        main.removeCallbacks(publishSoon)
        emulator.reset()
        paintGate.onSessionOpen()
        pendingAttachId = sessionId
        lastSentCols = 0
        lastSentRows = 0
        resizeQuietUntil = 0L
        publishEmpty()
        _state.update { it.copy(attachedId = sessionId) }
    }

    fun closeTerminal() {
        if (_state.value.attachedId != null) {
            client?.detach(TERM_CHANNEL)
        }
        main.removeCallbacks(proposeFlush)
        main.removeCallbacks(publishSoon)
        pendingAttachId = null
        resizeQuietUntil = 0L
        paintGate.onDetached()
        _state.update { it.copy(attachedId = null) }
    }

    fun sendInput(bytes: ByteArray) {
        client?.sendInput(TERM_CHANNEL, bytes)
    }

    fun proposeSize(cols: Int, rows: Int) {
        proposedCols = cols.coerceAtLeast(20)
        proposedRows = rows.coerceAtLeast(5)
        main.removeCallbacks(proposeFlush)
        main.postDelayed(proposeFlush, PROPOSE_DEBOUNCE_MS)
    }

    fun requestRename(session: SessionInfo) {
        _state.update { it.copy(pendingRename = session, renameDraft = session.name) }
    }

    fun onRenameDraft(value: String) {
        _state.update { it.copy(renameDraft = value) }
    }

    fun confirmRename() {
        val target = _state.value.pendingRename ?: return
        val name = _state.value.renameDraft.trim()
        if (name.isNotEmpty()) {
            client?.send(ClientMsg.RenameSession(target.id, name))
        }
        _state.update { it.copy(pendingRename = null, renameDraft = "") }
    }

    fun dismissRename() {
        _state.update { it.copy(pendingRename = null, renameDraft = "") }
    }

    fun killSession(id: Long) {
        if (_state.value.attachedId == id) closeTerminal()
        client?.send(ClientMsg.KillSession(id))
    }

    fun attachedSession(): SessionInfo? {
        val id = _state.value.attachedId ?: return null
        return _state.value.sessions.firstOrNull { it.id == id }
    }

    private fun handleServer(msg: ServerMsg) {
        when (msg) {
            is ServerMsg.Sessions -> _state.update { it.copy(sessions = msg.sessions) }
            is ServerMsg.SessionEvent -> {
                client?.send(ClientMsg.ListSessions)
                if (msg.event == ai.eclosion.octoterm.android.wire.SessionEventKind.Closed &&
                    _state.value.attachedId == msg.session.id
                ) {
                    closeTerminal()
                }
            }
            is ServerMsg.Attached -> {
                if (msg.channel == TERM_CHANNEL) {
                    paintGate.onAttached(replay = msg.mode == AttachMode.Replay)
                }
            }
            is ServerMsg.Resized -> {
                // 权威几何只改模拟器。提交画面会把「正确重绘」换成裁切后的空/错帧。
                if (msg.channel == TERM_CHANNEL) emulator.resize(msg.cols, msg.rows)
            }
            is ServerMsg.ResyncBegin -> {
                if (msg.channel == TERM_CHANNEL) {
                    paintGate.onResyncBegin()
                    emulator.reset()
                }
            }
            is ServerMsg.ResyncEnd -> {
                if (msg.channel == TERM_CHANNEL) {
                    paintGate.onResyncEnd()
                    main.removeCallbacks(publishSoon)
                    publishTerm()
                    // 首帧刚落地时 layout 还会再报一两次差 1 列的尺寸。
                    // 立刻 resize 会 SIGWINCH，对端先发 2J 清屏，空帧就会把
                    // 刚画好的重绘盖掉。
                    resizeQuietUntil = SystemClock.uptimeMillis() + RESIZE_QUIET_MS
                }
            }
            is ServerMsg.SessionExited -> {
                if (msg.channel == TERM_CHANNEL) closeTerminal()
                client?.send(ClientMsg.ListSessions)
            }
            is ServerMsg.Error -> {
                _messages.tryEmit(UserMessage.AuthFailed(msg.message))
                if (msg.channel == TERM_CHANNEL) closeTerminal()
            }
            else -> Unit
        }
    }

    private fun dropClient() {
        client?.close()
        client = null
    }

    private fun applyProposedSize() {
        main.removeCallbacks(proposeFlush)
        val pending = pendingAttachId
        val attached = _state.value.attachedId
        if (pending != null && attached == pending) {
            pendingAttachId = null
            lastSentCols = proposedCols
            lastSentRows = proposedRows
            client?.attach(pending, TERM_CHANNEL, proposedCols, proposedRows)
            return
        }
        if (attached != null && paintGate.shouldSendResize()) {
            val wait = resizeQuietUntil - SystemClock.uptimeMillis()
            if (wait > 0) {
                main.postDelayed(proposeFlush, wait)
                return
            }
            if (!significantResize(proposedCols, proposedRows, lastSentCols, lastSentRows)) return
            lastSentCols = proposedCols
            lastSentRows = proposedRows
            client?.resize(TERM_CHANNEL, proposedCols, proposedRows)
        }
    }

    private fun schedulePublish() {
        main.removeCallbacks(publishSoon)
        main.postDelayed(publishSoon, PUBLISH_COALESCE_MS)
    }

    private fun publishEmpty() {
        painted = TermFrame.Empty
        _state.update { it.copy(termGeneration = emulator.generation) }
    }

    private fun publishTerm() {
        val next = TermFrame.capture(emulator)
        // 服务端重绘以 2J 清屏开头。若清屏被单独提交，正确画面会被空网格盖住。
        if (!next.hasContent() && painted.hasContent()) return
        painted = next
        _state.update { it.copy(termGeneration = emulator.generation) }
    }

    private fun onMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else main.post(block)
    }

    fun connection(id: String): ServerConnection? = _state.value.connections.firstOrNull { it.id == id }

    private fun startEditor(connectionId: String?, prefillUrl: String?) {
        val existing = connectionId?.let { id -> _state.value.connections.firstOrNull { it.id == id } }
        val base = EditorState(
            name = existing?.name.orEmpty(),
            url = existing?.url.orEmpty(),
            token = existing?.token.orEmpty(),
            clipboardUrl = clipboardUrl(),
        )
        val withPrefill = if (existing == null && !prefillUrl.isNullOrBlank()) {
            applyPrefill(base, prefillUrl)
            return
        } else {
            refreshWarnings(base)
        }
        _state.update {
            it.copy(
                screen = AppScreen.Editor(connectionId),
                editor = withPrefill,
                showAddSheet = false,
            )
        }
    }

    private fun applyPrefill(base: EditorState, rawUrl: String) {
        val parsed = try {
            ConnectionUrl.parse(rawUrl)
        } catch (_: ConnectionUrlException) {
            _state.update {
                it.copy(
                    screen = AppScreen.Editor(null),
                    editor = refreshWarnings(base.copy(url = rawUrl)),
                    showAddSheet = false,
                )
            }
            return
        }
        val filled = refreshWarnings(
            base.copy(
                url = rawUrl.trim(),
                token = parsed.token ?: base.token,
                name = base.name.ifBlank { parsed.displayHost },
            ),
        )
        _state.update {
            it.copy(
                screen = AppScreen.Editor(null),
                editor = filled,
                showAddSheet = false,
            )
        }
    }

    private fun refreshWarnings(editor: EditorState): EditorState {
        val parsed = try {
            ConnectionUrl.parse(editor.url, editor.token)
        } catch (_: ConnectionUrlException) {
            return editor.copy(loopbackWarning = false, wildcardWarning = false)
        }
        return editor.copy(
            loopbackWarning = parsed.isLoopback,
            wildcardWarning = parsed.isWildcardBind,
        )
    }

    private fun persist(items: List<ServerConnection>) {
        store.save(items)
    }

    private fun clipboardUrl(): String? {
        val clipboard = app.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val text = clipboard.primaryClip
            ?.takeIf { it.itemCount > 0 }
            ?.getItemAt(0)
            ?.coerceToText(app)
            ?.toString()
            ?.trim()
            .orEmpty()
        if (text.isEmpty() || !ConnectionUrl.looksLikeUrl(text)) return null
        return text
    }

    override fun onCleared() {
        main.removeCallbacks(proposeFlush)
        main.removeCallbacks(publishSoon)
        dropClient()
        super.onCleared()
    }

    private fun sorted(items: List<ServerConnection>): List<ServerConnection> {
        return items.sortedWith(
            compareByDescending<ServerConnection> { it.lastUsedAt ?: 0L }
                .thenByDescending { it.createdAt },
        )
    }

    companion object {
        const val TERM_CHANNEL: Int = 1
        private const val PROPOSE_DEBOUNCE_MS = 80L
        private const val PUBLISH_COALESCE_MS = 32L
        private const val RESIZE_QUIET_MS = 800L

        internal fun significantResize(cols: Int, rows: Int, prevCols: Int, prevRows: Int): Boolean {
            if (prevCols <= 0 || prevRows <= 0) return true
            return abs(cols - prevCols) >= 2 || abs(rows - prevRows) >= 2
        }

        val Factory: ViewModelProvider.Factory = viewModelFactory {
            initializer {
                val app = checkNotNull(this[APPLICATION_KEY])
                ConnectionViewModel(app)
            }
        }
    }
}
