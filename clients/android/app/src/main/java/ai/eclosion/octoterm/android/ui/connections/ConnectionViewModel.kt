package ai.eclosion.octoterm.android.ui.connections

import android.app.Application
import android.content.ClipboardManager
import android.content.Context
import android.content.res.Configuration
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelProvider.AndroidViewModelFactory.Companion.APPLICATION_KEY
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import ai.eclosion.octoterm.android.agent.AgentSession
import ai.eclosion.octoterm.android.agent.AgentState
import ai.eclosion.octoterm.android.agent.AgentSessions
import ai.eclosion.octoterm.android.agent.AgentHistory
import ai.eclosion.octoterm.android.agent.HistoryEntry
import ai.eclosion.octoterm.android.agent.HistoryPage
import ai.eclosion.octoterm.android.agent.HistoryFailure
import ai.eclosion.octoterm.android.agent.Chat
import ai.eclosion.octoterm.android.agent.ChatFallback
import ai.eclosion.octoterm.android.agent.ChatMessage
import ai.eclosion.octoterm.android.agent.ChatWindow
import ai.eclosion.octoterm.android.appearance.AppearanceCodec
import ai.eclosion.octoterm.android.appearance.AppearanceStore
import ai.eclosion.octoterm.android.appearance.TermAppearance
import ai.eclosion.octoterm.android.appearance.TermFontKind
import ai.eclosion.octoterm.android.connection.CloudLink
import ai.eclosion.octoterm.android.connection.ConnectionStore
import ai.eclosion.octoterm.android.connection.ConnectionUrl
import ai.eclosion.octoterm.android.connection.ConnectionUrlException
import ai.eclosion.octoterm.android.connection.ServerConnection
import ai.eclosion.octoterm.android.launcher.Launcher
import ai.eclosion.octoterm.android.launcher.Launchers
import ai.eclosion.octoterm.android.net.OctoClient
import ai.eclosion.octoterm.android.push.AgentNotifications
import ai.eclosion.octoterm.android.push.AgentPush
import ai.eclosion.octoterm.android.term.TermFrame
import ai.eclosion.octoterm.android.term.AgentKeyLayout
import ai.eclosion.octoterm.android.term.TermPaintGate
import ai.eclosion.octoterm.android.term.TermSelection
import ai.eclosion.octoterm.android.term.VtEmulator
import ai.eclosion.octoterm.android.wire.AttachMode
import ai.eclosion.octoterm.android.wire.ClientMsg
import ai.eclosion.octoterm.android.wire.ServerMsg
import ai.eclosion.octoterm.android.wire.SessionInfo
import ai.eclosion.octoterm.android.wire.TermColors
import android.content.ClipData
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import kotlin.math.abs
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import java.util.UUID
import java.util.concurrent.TimeUnit

sealed class AppScreen {
    data object List : AppScreen()
    data class Editor(val connectionId: String?) : AppScreen()
    data class Server(val connectionId: String) : AppScreen()
    data object Appearance : AppScreen()
    data object CloudAccount : AppScreen()
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
    data object Copied : UserMessage()
    data object ImportFailed : UserMessage()
    data object TooManySessions : UserMessage()
    data object ConnectServerFirst : UserMessage()
    data object SessionGone : UserMessage()
}

data class LauncherMenuState(
    val open: Boolean = false,
    val loading: Boolean = false,
    val items: List<Launcher> = emptyList(),
)

/** 聊天视图里没有消息可看时,说明为什么。 */
sealed class ChatNotice {
    /** 这个会话里没认出 agent */
    data object NoAgent : ChatNotice()
    /** 服务端说读不了,带上是哪一家,好让文案说清楚 */
    data class Server(val reason: ChatFallback, val agentId: String) : ChatNotice()
}

/**
 * 聊天视图。`sessionId` 记的是**为哪个会话打开的** —— 只有它等于当前会话时才显示,
 * 所以切标签、关会话时不需要到处同步一个开关。
 */
data class ChatUiState(
    val sessionId: Long? = null,
    val messages: List<ChatMessage> = emptyList(),
    val notice: ChatNotice? = null,
    val loading: Boolean = false,
)

data class ConnectionUiState(
    val connections: List<ServerConnection> = emptyList(),
    val screen: AppScreen = AppScreen.List,
    val editor: EditorState = EditorState(),
    val connectingId: String? = null,
    val pendingDelete: ServerConnection? = null,
    val showAddSheet: Boolean = false,
    val sessions: List<SessionInfo> = emptyList(),
    val openSessionIds: List<Long> = emptyList(),
    val attachedId: Long? = null,
    val reconnecting: Boolean = false,
    val serverOs: String = "",
    val pendingRename: SessionInfo? = null,
    val renameDraft: String = "",
    val termGeneration: Long = 0,
    val launcherMenu: LauncherMenuState = LauncherMenuState(),
    val appearance: TermAppearance = AppearanceCodec.default(),
    val appearanceFollowsSystem: Boolean = true,
    val extraKeyRows: Int = 2,
    val agentKeyLayouts: Map<String, AgentKeyLayout> = emptyMap(),
    val selection: TermSelection? = null,
    val scrollOffset: Int = 0,
    val mouseTracking: Boolean = false,
    /** key = "agent_id agent_session_id",见 `AgentSession.key` */
    val agents: Map<String, AgentSession> = emptyMap(),
    /** 服务端是否允许读对话记录。只认快照里明确的 true */
    val transcriptEnabled: Boolean = false,
    val chat: ChatUiState = ChatUiState(),
) {
    val chatOpen: Boolean get() = chat.sessionId != null && chat.sessionId == attachedId
    val chatAvailable: Boolean get() = AgentSessions.chatAvailable(agents, attachedId, transcriptEnabled)
}

class ConnectionViewModel(
    private val app: Application,
) : ViewModel() {
    private val store = ConnectionStore(app)
    private val appearanceStore = AppearanceStore(app)
    private var systemDark = app.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK ==
        Configuration.UI_MODE_NIGHT_YES
    private val main = Handler(Looper.getMainLooper())
    private val http = OkHttpClient.Builder().callTimeout(8, TimeUnit.SECONDS).build()
    private var client: OctoClient? = null
    private val opens = LinkedHashMap<Long, OpenSession>()
    private var activeId: Long? = null
    private var httpOrigin: String? = null
    private var httpToken: String? = null
    private var awaitingCreated = false
    /** 点通知要去的会话;连上并拿到会话列表后消费,连接失败/断开即作废。 */
    private val pendingOpen = PendingOpenTracker()
    /** 每次发起连接 +1:旧连接的握手超时不能误伤用户之后发起的新连接。 */
    private var connectAttempt = 0L
    private var agentJob: Job? = null
    private var agentAgain = false
    /** 每次 agent 表被事件改动就 +1:拉取期间到达的事件不能被较旧的快照覆盖 */
    private var agentRevision = 0L
    private var chatJob: Job? = null
    private var chatAgain = false
    private var chatCursor: String? = null
    /** 当前消息属于哪个 agent 会话。换了 agent 就得整段重拉,旧游标对新记录没有意义 */
    private var chatAgentKey: String? = null
    @Volatile
    var painted: TermFrame = TermFrame.Empty
        private set
    private var proposedCols = 80
    private var proposedRows = 24
    private val proposeFlush = Runnable { applyProposedSize() }
    private val publishSoon = Runnable { publishTerm() }
    private val _state = MutableStateFlow(
        ConnectionUiState(
            connections = store.load(),
            appearance = appearanceStore.load(systemDark),
            appearanceFollowsSystem = appearanceStore.followsSystem(),
            extraKeyRows = appearanceStore.extraKeyRows(),
            agentKeyLayouts = appearanceStore.agentKeyLayouts(),
        ),
    )
    val state: StateFlow<ConnectionUiState> = _state.asStateFlow()

    // 用 Channel 而不是 replay=0 的 SharedFlow:冷启动点通知时 handleIntent 先于 setContent,
    // 那时还没有收集者,一次性提示会直接丢失;Channel 会缓存到界面开始收集为止。
    // 容量限定为 4 并丢最旧的:突发事件不该堆积成几十条迟到很久才弹出的 snackbar。
    private val _messages = Channel<UserMessage>(capacity = 4, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    val messages: Flow<UserMessage> = _messages.receiveAsFlow()

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

    fun openAppearance() {
        _state.update { it.copy(screen = AppScreen.Appearance) }
    }

    fun closeAppearance() {
        _state.update { it.copy(screen = AppScreen.List) }
    }

    fun openCloudAccount() {
        _state.update { it.copy(screen = AppScreen.CloudAccount) }
    }

    fun closeCloudAccount() {
        _state.update { it.copy(screen = AppScreen.List) }
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
                connections = updated,
                screen = AppScreen.List,
                editor = EditorState(clipboardUrl = clipboardUrl()),
            )
        }
        _messages.trySend(UserMessage.Saved)
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
        _messages.trySend(UserMessage.Deleted)
    }

    fun connect(id: String) {
        // 每个提前返回都作废点通知的跳转:它只属于为它发起的那次连接。
        val connection = _state.value.connections.firstOrNull { it.id == id } ?: run {
            pendingOpen.onConnectFailed()
            return
        }
        if (_state.value.connectingId != null) {
            pendingOpen.onConnectFailed()
            return
        }
        val parsed = try {
            ConnectionUrl.parse(connection.url, connection.token)
        } catch (_: ConnectionUrlException) {
            pendingOpen.onConnectFailed()
            _messages.trySend(UserMessage.Unexpected)
            return
        }
        val token = parsed.token
        if (token.isNullOrEmpty()) {
            pendingOpen.onConnectFailed()
            _messages.trySend(UserMessage.AuthFailed(null))
            return
        }
        pendingOpen.onConnectStart(id)
        val attempt = ++connectAttempt
        httpOrigin = parsed.httpOrigin()
        httpToken = token
        _state.update { it.copy(connectingId = id, serverOs = "") }
        val next = OctoClient()
        client?.close()
        client = next
        next.onOpen = {
            onMain {
                // 已被取消/替换的连接迟到的回调不能再切屏。
                if (client !== next) return@onMain
                val now = System.currentTimeMillis()
                val updated = _state.value.connections.map { item ->
                    if (item.id == id) item.copy(lastUsedAt = now) else item
                }
                persist(updated)
                _state.update {
                    it.copy(
                        connections = updated,
                        connectingId = null,
                        reconnecting = false,
                        screen = AppScreen.Server(id),
                    )
                }
                next.send(ClientMsg.ListSessions)
                // 断线期间漏掉的 agent-event 靠这次全量拉取补齐(协议 A5),不做增量对账
                agentRevision++
                refreshAgents()
            }
        }
        next.onReconnecting = { onMain { _state.update { it.copy(reconnecting = true) } } }
        next.onFatal = { message ->
            onMain {
                if (client !== next) return@onMain
                pendingOpen.onConnectFailed()
                closeAllSessions()
                _state.update { it.copy(connectingId = null, reconnecting = false) }
                dropClient()
                _messages.trySend(UserMessage.AuthFailed(message))
                if (_state.value.screen is AppScreen.Server) {
                    _state.update {
                        it.copy(
                            screen = AppScreen.List,
                            sessions = emptyList(),
                            attachedId = null,
                            openSessionIds = emptyList(),
                        )
                    }
                }
            }
        }
        next.onControl = { msg ->
            onMain {
                if (client !== next) return@onMain
                if (msg is ServerMsg.HelloOk) {
                    _state.update { it.copy(serverOs = msg.os) }
                    CloudLink.record(_state.value.connections, id, msg.cloudServerId)?.let { updated ->
                        persist(updated)
                        _state.update { it.copy(connections = updated) }
                    }
                }
                handleServer(msg)
            }
        }
        next.onChannelData = { channel, payload ->
            // 必须和 control 同一条主线程队列、按到达顺序处理。
            // 若在 OkHttp 线程 write、主线程 resync-begin/reset，重绘字节会被后到的 reset 清掉。
            onMain {
                val open = opens.values.firstOrNull { it.channel == channel } ?: return@onMain
                open.emulator.write(payload)
                next.noteData(channel, payload.size)
                if (open.id == activeId && open.paintGate.shouldPublishWrite()) schedulePublish()
            }
        }
        next.connect(parsed.webSocketUrl, token)
        viewModelScope.launch {
            // Handshake timeout: if still connecting, fail.
            kotlinx.coroutines.delay(12_000)
            if (connectAttempt == attempt && _state.value.connectingId == id && _state.value.screen !is AppScreen.Server) {
                pendingOpen.onConnectFailed()
                dropClient()
                _state.update { it.copy(connectingId = null) }
                _messages.trySend(UserMessage.Timeout)
            }
        }
    }

    fun disconnect() {
        android.util.Log.i("OctotermPush", "Terminal connection disconnected")
        pendingOpen.onDisconnect()
        resetChat()
        agentJob?.cancel()
        agentJob = null
        closeAllSessions()
        dropClient()
        httpOrigin = null
        httpToken = null
        _state.update {
            it.copy(
                screen = AppScreen.List,
                sessions = emptyList(),
                attachedId = null,
                openSessionIds = emptyList(),
                reconnecting = false,
                serverOs = "",
                agents = emptyMap(),
                transcriptEnabled = false,
                chat = ChatUiState(),
                pendingRename = null,
                launcherMenu = LauncherMenuState(),
                selection = null,
                scrollOffset = 0,
                mouseTracking = false,
            )
        }
    }

    fun openFromNotification(cloudServerId: String, session: Long, agentId: String?) {
        val conn = CloudLink.find(_state.value.connections, cloudServerId) ?: run {
            pendingOpen.onConnectFailed()
            _messages.trySend(UserMessage.ConnectServerFirst)
            return
        }
        val target = PendingOpen(conn.id, session, agentId?.let { AgentPush.tag(cloudServerId, session, it) })
        val st = _state.value
        val screen = st.screen
        if (screen is AppScreen.Server && screen.connectionId == conn.id && st.connectingId == null) {
            pendingOpen.onTap(target)
            client?.send(ClientMsg.ListSessions) // 列表回来后在 handleServer 里打开
            return
        }
        // 用户最新的意图优先:放弃正在进行的连接/已连着的别的服务器,改连目标。
        if (st.connectingId != null) cancelConnecting()
        if (screen is AppScreen.Server) disconnect()
        pendingOpen.onTap(target)
        connect(conn.id)
    }

    /** 放弃正在握手的连接;作废它的超时计时。 */
    private fun cancelConnecting() {
        connectAttempt++
        dropClient()
        _state.update { it.copy(connectingId = null) }
    }

    private fun consumePendingOpen() {
        when (val decision = pendingOpen.onSessions(_state.value.screen, _state.value.sessions)) {
            null -> Unit
            is PendingOpenDecision.Open -> {
                openSession(decision.session)
                // 真打开了才撤回通知;打不开(会话太多等)就留着,用户还能再点。
                if (opens.containsKey(decision.session)) {
                    decision.notificationTag?.let { AgentNotifications.cancel(app, it) }
                }
            }
            PendingOpenDecision.SessionGone -> _messages.trySend(UserMessage.SessionGone)
        }
    }

    fun openLauncherMenu() {
        val origin = httpOrigin
        val token = httpToken
        if (origin.isNullOrEmpty() || token.isNullOrEmpty()) return
        _state.update {
            it.copy(launcherMenu = LauncherMenuState(open = true, loading = true, items = listOf(Launchers.fallback())))
        }
        viewModelScope.launch {
            val items = withContext(Dispatchers.IO) { Launchers.fetch(http, origin, token) }
            if (!_state.value.launcherMenu.open) return@launch
            _state.update { it.copy(launcherMenu = LauncherMenuState(open = true, loading = false, items = items)) }
        }
    }

    fun dismissLauncherMenu() {
        _state.update { it.copy(launcherMenu = LauncherMenuState()) }
    }

    fun newSession(launcher: Launcher) {
        _state.update { it.copy(launcherMenu = LauncherMenuState()) }
        awaitingCreated = true
        val command = launcher.command.takeIf { it.isNotEmpty() }
        client?.send(
            ClientMsg.NewSession(
                name = launcher.name.ifBlank { null },
                command = command,
                cwd = launcher.cwd,
            ),
        )
    }

    suspend fun loadHistory(query: String, agent: String, offset: Int, refresh: Boolean, workspace: String): HistoryPage {
        val origin = httpOrigin ?: throw HistoryFailure("unreadable")
        val token = httpToken ?: throw HistoryFailure("unreadable")
        return withContext(Dispatchers.IO) { AgentHistory.fetch(http, origin, token, query, agent, offset, refresh, workspace) }
    }

    suspend fun previewHistory(entry: HistoryEntry): ChatWindow {
        val origin = httpOrigin ?: throw HistoryFailure("unreadable")
        val token = httpToken ?: throw HistoryFailure("unreadable")
        return withContext(Dispatchers.IO) { AgentHistory.messages(http, origin, token, entry) }
    }

    suspend fun resumeHistory(entry: HistoryEntry) {
        val connection = client ?: throw HistoryFailure("unreadable")
        if (_state.value.reconnecting) throw HistoryFailure("unreadable")
        val origin = httpOrigin ?: throw HistoryFailure("unreadable")
        val token = httpToken ?: throw HistoryFailure("unreadable")
        val session = withContext(Dispatchers.IO) { AgentHistory.open(http, origin, token, entry) }
        if (client !== connection || _state.value.screen !is AppScreen.Server) return
        _state.update { state -> state.copy(sessions = (state.sessions + session).distinctBy { it.id }) }
        if (session.id !in opens && opens.size >= MAX_OPEN) {
            // Make room in the local render cache; the detached remote terminal keeps running.
            opens.keys.firstOrNull { it != activeId }?.let(::closeOpenSession)
        }
        openSession(session.id)
        connection.send(ClientMsg.ListSessions)
        agentRevision++
        refreshAgents()
    }

    fun openSession(sessionId: Long) {
        if (_state.value.chat.sessionId != sessionId) resetChat()
        val existing = opens[sessionId]
        if (existing != null) {
            activeId = sessionId
            _state.update { it.copy(selection = null) }
            publishTerm()
            return
        }
        if (opens.size >= MAX_OPEN) {
            _messages.trySend(UserMessage.TooManySessions)
            return
        }
        val open = OpenSession(
            id = sessionId,
            channel = nextChannel(),
            scrollback = _state.value.appearance.scrollback,
        )
        open.paintGate.onSessionOpen()
        opens[sessionId] = open
        activeId = sessionId
        painted = TermFrame.Empty
        _state.update {
            it.copy(
                attachedId = sessionId,
                openSessionIds = opens.keys.toList(),
                selection = null,
                scrollOffset = 0,
                mouseTracking = false,
                termGeneration = open.emulator.generation,
            )
        }
        main.removeCallbacks(proposeFlush)
        main.post(proposeFlush)
    }

    /** 回到会话列表，已经打开的会话保持 attach。 */
    fun leaveTerminal() {
        resetChat()
        activeId = null
        _state.update { it.copy(attachedId = null, selection = null) }
    }

    fun closeOpenSession(id: Long) {
        val open = opens.remove(id) ?: return
        if (_state.value.chat.sessionId == id) resetChat()
        client?.detach(open.channel)
        open.paintGate.onDetached()
        if (activeId == id) {
            activeId = opens.keys.lastOrNull()
            if (activeId == null) {
                painted = TermFrame.Empty
                _state.update {
                    it.copy(
                        attachedId = null,
                        openSessionIds = emptyList(),
                        selection = null,
                        scrollOffset = 0,
                        mouseTracking = false,
                    )
                }
            } else {
                publishTerm()
            }
        } else {
            _state.update { it.copy(openSessionIds = opens.keys.toList()) }
        }
    }

    fun activeEmulator(): VtEmulator? = opens[activeId]?.emulator

    fun sendInput(bytes: ByteArray) {
        sendInput(activeId ?: return, bytes)
    }

    fun sendInput(sessionId: Long, bytes: ByteArray) {
        if (_state.value.reconnecting || bytes.isEmpty()) return
        val open = opens[sessionId] ?: return
        open.emulator.scrollToBottom()
        client?.sendInput(open.channel, bytes)
        _state.update { it.copy(selection = null) }
        publishTerm()
    }

    fun sendMouse(col: Int, row: Int, pressed: Boolean, moving: Boolean) {
        val open = opens[activeId] ?: return
        if (open.emulator.scrollOffset != 0) open.emulator.scrollToBottom()
        val bytes = open.emulator.encodePointer(col, row, button = 0, pressed = pressed, moving = moving) ?: return
        if (bytes.isEmpty()) return
        client?.sendInput(open.channel, bytes)
    }

    fun scrollBy(delta: Int) {
        val open = opens[activeId] ?: return
        open.emulator.scrollBy(delta)
        _state.update { it.copy(selection = null) }
        publishTerm(allowEmpty = true)
    }

    fun jumpToBottom() {
        val open = opens[activeId] ?: return
        open.emulator.scrollToBottom()
        publishTerm(allowEmpty = true)
    }

    fun setSelection(selection: TermSelection?) {
        _state.update { it.copy(selection = selection) }
    }

    fun copySelection() {
        val open = opens[activeId] ?: return
        val selection = _state.value.selection ?: return
        val text = open.emulator.copyViewport(selection.x0, selection.y0, selection.x1, selection.y1)
        if (text.isEmpty()) return
        val clipboard = app.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("octoterm", text))
        _messages.trySend(UserMessage.Copied)
    }

    fun pasteClipboard() {
        val open = opens[activeId] ?: return
        val clipboard = app.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val text = clipboard.primaryClip
            ?.takeIf { it.itemCount > 0 }
            ?.getItemAt(0)
            ?.coerceToText(app)
            ?.toString()
            .orEmpty()
        if (text.isEmpty()) return
        open.emulator.scrollToBottom()
        client?.sendInput(open.channel, open.emulator.encodePaste(text))
        _state.update { it.copy(selection = null) }
        publishTerm()
    }

    fun setFontSize(size: Float) = updateAppearance(_state.value.appearance.withFontSize(size))

    fun setExtraKeyRows(rows: Int) {
        val next = rows.coerceIn(1, 2)
        appearanceStore.saveExtraKeyRows(next)
        _state.update { it.copy(extraKeyRows = next) }
    }

    fun setAgentKeyLayout(agentId: String, layout: AgentKeyLayout) {
        val next = _state.value.agentKeyLayouts.toMutableMap()
        if (layout == AgentKeyLayout()) next.remove(agentId) else next[agentId] = layout
        appearanceStore.saveAgentKeyLayouts(next)
        _state.update { it.copy(agentKeyLayouts = next) }
    }

    fun setFont(kind: TermFontKind) = updateAppearance(_state.value.appearance.withFont(kind))

    fun setScrollback(lines: Int) = updateAppearance(_state.value.appearance.withScrollback(lines))

    fun onSystemThemeChanged(prefersDark: Boolean) {
        systemDark = prefersDark
        _state.update {
            if (it.appearanceFollowsSystem) {
                it.copy(appearance = it.appearance.withTheme(AppearanceCodec.default(prefersDark)))
            } else it
        }
        reportColors()
    }

    fun followSystemTheme() {
        updateAppearance(
            _state.value.appearance.withTheme(AppearanceCodec.default(systemDark)),
            followsSystem = true,
        )
    }

    fun selectTheme(name: String) {
        val theme = AppearanceCodec.builtin(name) ?: return
        val current = _state.value.appearance
        updateAppearance(
            current.withTheme(theme),
            followsSystem = false,
        )
    }

    fun exportAppearance(): String = AppearanceCodec.exportJson(_state.value.appearance)

    fun importAppearance(text: String) {
        val next = AppearanceCodec.importJson(text, _state.value.appearance)
        if (next == null) {
            _messages.trySend(UserMessage.ImportFailed)
            return
        }
        updateAppearance(next, followsSystem = false)
        _messages.trySend(UserMessage.Saved)
    }

    fun copyAppearance() {
        val clipboard = app.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("octoterm-appearance", exportAppearance()))
        _messages.trySend(UserMessage.Copied)
    }

    fun importAppearanceFromClipboard() {
        val clipboard = app.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        val text = clipboard.primaryClip
            ?.takeIf { it.itemCount > 0 }
            ?.getItemAt(0)
            ?.coerceToText(app)
            ?.toString()
            .orEmpty()
        if (text.isBlank()) {
            _messages.trySend(UserMessage.ImportFailed)
            return
        }
        importAppearance(text)
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
        if (opens.containsKey(id)) closeOpenSession(id)
        client?.send(ClientMsg.KillSession(id))
    }

    fun attachedSession(): SessionInfo? {
        val id = _state.value.attachedId ?: return null
        return _state.value.sessions.firstOrNull { it.id == id }
    }

    fun openTabs(): List<SessionInfo> {
        val byId = _state.value.sessions.associateBy { it.id }
        return opens.keys.map { id -> byId[id] ?: SessionInfo(id, "", 0, 0, 0) }
    }

    private fun handleServer(msg: ServerMsg) {
        when (msg) {
            is ServerMsg.Sessions -> {
                _state.update { it.copy(sessions = msg.sessions) }
                consumePendingOpen()
            }
            is ServerMsg.SessionEvent -> {
                client?.send(ClientMsg.ListSessions)
                if (msg.event == ai.eclosion.octoterm.android.wire.SessionEventKind.Closed) {
                    if (opens.containsKey(msg.session.id)) closeOpenSession(msg.session.id)
                } else if (msg.event == ai.eclosion.octoterm.android.wire.SessionEventKind.Created && awaitingCreated) {
                    awaitingCreated = false
                    openSession(msg.session.id)
                }
            }
            is ServerMsg.Attached -> {
                opens.values.firstOrNull { it.channel == msg.channel }
                    ?.paintGate
                    ?.onAttached(replay = msg.mode == AttachMode.Replay)
            }
            is ServerMsg.Resized -> {
                // 权威几何只改模拟器。提交画面会把「正确重绘」换成裁切后的空/错帧。
                opens.values.firstOrNull { it.channel == msg.channel }
                    ?.emulator
                    ?.resize(msg.cols, msg.rows)
            }
            is ServerMsg.ResyncBegin -> {
                val open = opens.values.firstOrNull { it.channel == msg.channel } ?: return
                open.paintGate.onResyncBegin()
                open.emulator.reset()
                open.emulator.setMaxScrollback(_state.value.appearance.scrollback)
            }
            is ServerMsg.ResyncEnd -> {
                val open = opens.values.firstOrNull { it.channel == msg.channel } ?: return
                open.paintGate.onResyncEnd()
                if (open.id == activeId) {
                    main.removeCallbacks(publishSoon)
                    publishTerm()
                }
                // 首帧刚落地时 layout 还会再报一两次差 1 列的尺寸。
                // 立刻 resize 会 SIGWINCH，对端先发 2J 清屏，空帧就会把
                // 刚画好的重绘盖掉。
                open.resizeQuietUntil = SystemClock.uptimeMillis() + RESIZE_QUIET_MS
            }
            is ServerMsg.SessionExited -> {
                val open = opens.values.firstOrNull { it.channel == msg.channel }
                if (open != null) closeOpenSession(open.id)
                client?.send(ClientMsg.ListSessions)
            }
            is ServerMsg.AgentEvent -> onAgentEvent(msg.session)
            is ServerMsg.Error -> {
                _messages.trySend(UserMessage.AuthFailed(msg.message))
                val channel = msg.channel
                if (channel != null) {
                    val open = opens.values.firstOrNull { it.channel == channel }
                    if (open != null) closeOpenSession(open.id)
                }
            }
            else -> Unit
        }
    }

    /** 聊天 ⇄ 终端。只有本会话有读得了记录的 agent 时才能进聊天;回终端永远可以。 */
    fun toggleChat() {
        val st = _state.value
        if (st.chatOpen) {
            resetChat()
            return
        }
        val sessionId = st.attachedId ?: return
        if (!st.chatAvailable) return
        _state.update { it.copy(chat = ChatUiState(sessionId = sessionId, loading = true)) }
        // 进聊天视图才去拉 —— 不看的时候不该白读别人的文件
        refreshChat(incremental = false)
    }

    private fun onAgentEvent(ev: AgentSession) {
        agentRevision++
        val agents = AgentSessions.applyEvent(_state.value.agents, ev)
        _state.update { it.copy(agents = agents) }
        // 新出现的 agent 还不知道能不能读记录 —— 事件不带这一项,要快照来补
        if (agents[ev.key]?.supportsTranscript == null && ev.state != AgentState.Done) {
            refreshAgents()
        }
        closeChatIfUnavailable()
        // 有事件才可能有新消息 —— 所以不做轮询
        if (_state.value.chatOpen && ev.session == _state.value.chat.sessionId) refreshChat(incremental = true)
    }

    /** 全量快照。并发的请求合并成「再拉一次」,不叠加。 */
    private fun refreshAgents() {
        if (agentJob?.isActive == true) {
            agentAgain = true
            return
        }
        val origin = httpOrigin ?: return
        val token = httpToken ?: return
        agentJob = viewModelScope.launch {
            do {
                agentAgain = false
                val revision = agentRevision
                val snapshot = withContext(Dispatchers.IO) { AgentSessions.fetch(http, origin, token) }
                if (revision != agentRevision) {
                    agentAgain = true
                    continue
                }
                _state.update {
                    it.copy(
                        agents = AgentSessions.replaceAll(snapshot.sessions),
                        transcriptEnabled = snapshot.transcriptEnabled,
                    )
                }
                closeChatIfUnavailable()
                // 重连期间可能漏了事件:开着聊天就补拉一次
                if (_state.value.chatOpen) refreshChat(incremental = true)
            } while (agentAgain)
        }
    }

    private fun closeChatIfUnavailable() {
        val st = _state.value
        if (st.chat.sessionId != null && !st.chatAvailable) resetChat()
    }

    private fun resetChat() {
        chatJob?.cancel()
        chatJob = null
        chatAgain = false
        chatCursor = null
        chatAgentKey = null
        if (_state.value.chat != ChatUiState()) _state.update { it.copy(chat = ChatUiState()) }
    }

    /**
     * 拉一次消息。`incremental` 为真时带上游标;拉取串行化,期间再来的请求合并成「再拉一次」。
     *
     * 服务端说 `more` 就接着拉 —— 那表示被字节上限截断了。但**只有游标真的动了才继续**:
     * 游标原地不动还接着拉就是死循环(web 端复现过)。
     */
    private fun refreshChat(incremental: Boolean) {
        if (chatJob?.isActive == true) {
            chatAgain = true
            return
        }
        val origin = httpOrigin ?: return
        val token = httpToken ?: return
        chatJob = viewModelScope.launch {
            var inc = incremental
            do {
                chatAgain = false
                val st = _state.value
                val sessionId = st.chat.sessionId
                if (sessionId == null || !st.chatOpen) return@launch
                val agent = AgentSessions.chatAgentFor(st.agents, sessionId)
                if (agent == null) {
                    // 说清楚,并给一条出路 —— 沉默会让人以为是坏了
                    _state.update { it.copy(chat = it.chat.copy(notice = ChatNotice.NoAgent, loading = false)) }
                    return@launch
                }
                if (agent.key != chatAgentKey) inc = false
                val after = if (inc) chatCursor else null
                val win = withContext(Dispatchers.IO) {
                    Chat.fetch(http, origin, token, agent.agentId, agent.agentSessionId, after)
                }
                val now = _state.value
                if (now.chat.sessionId != sessionId || !now.chatOpen) return@launch
                if (AgentSessions.chatAgentFor(now.agents, sessionId)?.key != agent.key) {
                    // 拉取期间换了 agent:这份结果作废,从头拉新的那个
                    chatAgain = true
                    continue
                }
                when (win) {
                    is ChatWindow.Terminal -> _state.update {
                        it.copy(chat = it.chat.copy(notice = ChatNotice.Server(win.reason, agent.agentId), loading = false))
                    }
                    is ChatWindow.Transcript -> {
                        val advanced = !inc || win.cursor != chatCursor
                        chatCursor = win.cursor
                        chatAgentKey = agent.key
                        _state.update {
                            it.copy(
                                chat = it.chat.copy(
                                    messages = Chat.merge(if (inc) it.chat.messages else emptyList(), win),
                                    notice = null,
                                    loading = false,
                                ),
                            )
                        }
                        if (win.more && advanced) chatAgain = true
                    }
                }
                inc = true
            } while (chatAgain)
        }
    }

    private fun dropClient() {
        client?.close()
        client = null
    }

    private fun applyProposedSize() {
        main.removeCallbacks(proposeFlush)
        var retryIn = Long.MAX_VALUE
        for (open in opens.values) {
            if (open.pendingAttach) {
                open.pendingAttach = false
                open.lastSentCols = proposedCols
                open.lastSentRows = proposedRows
                client?.attach(open.id, open.channel, proposedCols, proposedRows, termColors())
                continue
            }
            if (!open.paintGate.shouldSendResize()) continue
            val wait = open.resizeQuietUntil - SystemClock.uptimeMillis()
            if (wait > 0) {
                retryIn = minOf(retryIn, wait)
                continue
            }
            if (!significantResize(proposedCols, proposedRows, open.lastSentCols, open.lastSentRows)) continue
            open.lastSentCols = proposedCols
            open.lastSentRows = proposedRows
            client?.resize(open.channel, proposedCols, proposedRows)
        }
        if (retryIn != Long.MAX_VALUE) main.postDelayed(proposeFlush, retryIn.coerceAtLeast(1))
    }

    private fun schedulePublish() {
        main.removeCallbacks(publishSoon)
        main.postDelayed(publishSoon, PUBLISH_COALESCE_MS)
    }

    private fun publishTerm(allowEmpty: Boolean = false) {
        val open = opens[activeId] ?: return
        val next = TermFrame.capture(open.emulator)
        // 服务端重绘以 2J 清屏开头。若清屏被单独提交，正确画面会被空网格盖住。
        // 用户自己翻历史时允许换成另一块（可能更空的）画面。
        if (!allowEmpty && !next.hasContent() && open.painted.hasContent() && next.scrollOffset == open.painted.scrollOffset) {
            return
        }
        open.painted = next
        painted = next
        _state.update {
            it.copy(
                attachedId = activeId,
                openSessionIds = opens.keys.toList(),
                termGeneration = open.emulator.generation,
                scrollOffset = open.emulator.scrollOffset,
                mouseTracking = open.emulator.mouseTracking,
                selection = if (allowEmpty) null else it.selection,
            )
        }
    }

    private fun closeAllSessions() {
        for (open in opens.values) {
            client?.detach(open.channel)
            open.paintGate.onDetached()
        }
        opens.clear()
        activeId = null
        awaitingCreated = false
        painted = TermFrame.Empty
    }

    private fun nextChannel(): Int {
        val used = opens.values.map { it.channel }.toSet()
        var channel = 1
        while (channel in used) channel++
        return channel
    }

    private fun updateAppearance(
        next: TermAppearance,
        followsSystem: Boolean = _state.value.appearanceFollowsSystem,
    ) {
        appearanceStore.save(next, followsSystem)
        for (open in opens.values) open.emulator.setMaxScrollback(next.scrollback)
        _state.update { it.copy(appearance = next, appearanceFollowsSystem = followsSystem) }
        if (activeId != null) publishTerm(allowEmpty = true)
        reportColors()
    }

    private fun termColors(): TermColors =
        _state.value.appearance.let { TermColors.fromRgb(it.foreground, it.background) }

    /** 服务端拿这个应答 OSC 10/11 颜色查询,主题一变就得重报。 */
    private fun reportColors() {
        val colors = termColors()
        for (open in opens.values) client?.setColors(open.channel, colors)
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

    companion object {
        private const val MAX_OPEN = 8
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

private class OpenSession(
    val id: Long,
    val channel: Int,
    scrollback: Int,
) {
    val emulator = VtEmulator(maxScrollback = scrollback)
    val paintGate = TermPaintGate()
    var painted: TermFrame = TermFrame.Empty
    var pendingAttach: Boolean = true
    var lastSentCols: Int = 0
    var lastSentRows: Int = 0
    var resizeQuietUntil: Long = 0L
}
