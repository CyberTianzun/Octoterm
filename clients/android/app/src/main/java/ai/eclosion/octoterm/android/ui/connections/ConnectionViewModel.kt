package ai.eclosion.octoterm.android.ui.connections

import android.app.Application
import android.content.ClipboardManager
import android.content.Context
import android.content.res.Configuration
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelProvider.AndroidViewModelFactory.Companion.APPLICATION_KEY
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
import ai.eclosion.octoterm.android.connection.ServerConnectionStatus
import ai.eclosion.octoterm.android.connection.ParsedEndpoint
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
    data object BackgroundUnavailable : UserMessage()
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
    val olderCursor: String? = null,
    val olderLoading: Boolean = false,
    val olderFailed: Boolean = false,
    val historySupported: Boolean = false,
    val historyPage: Int = 0,
)

data class ConnectionUiState(
    val connections: List<ServerConnection> = emptyList(),
    val screen: AppScreen = AppScreen.List,
    val editor: EditorState = EditorState(),
    val connectingId: String? = null,
    val connectionStatuses: Map<String, ServerConnectionStatus> = emptyMap(),
    val keepConnectionsInBackground: Boolean = true,
    val connectionServiceRunning: Boolean = false,
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

interface ConnectionActions {
    val state: StateFlow<ConnectionUiState>
    val messages: Flow<UserMessage>
    val painted: TermFrame
    fun openAddSheet()
    fun dismissAddSheet()
    fun addFromClipboard()
    fun addManually()
    fun edit(id: String)
    fun closeEditor()
    fun openAppearance()
    fun closeAppearance()
    fun openCloudAccount()
    fun closeCloudAccount()
    fun onNameChange(value: String)
    fun onUrlChange(value: String)
    fun onTokenChange(value: String)
    fun pasteClipboardIntoEditor()
    fun saveEditor()
    fun requestDelete(connection: ServerConnection)
    fun dismissDelete()
    fun confirmDelete()
    fun connect(id: String)
    fun leaveServer()
    fun openFromNotification(cloudServerId: String, session: Long, agentId: String?)
    fun openLauncherMenu()
    fun dismissLauncherMenu()
    fun newSession(launcher: Launcher)
    suspend fun loadHistory(query: String, agent: String, offset: Int, refresh: Boolean, workspace: String): HistoryPage
    suspend fun previewHistory(entry: HistoryEntry): ChatWindow
    suspend fun resumeHistory(entry: HistoryEntry)
    fun openSession(sessionId: Long)
    fun leaveTerminal()
    fun closeOpenSession(id: Long)
    fun activeEmulator(): VtEmulator?
    fun sendInput(bytes: ByteArray)
    fun sendInput(sessionId: Long, bytes: ByteArray)
    fun sendMouse(col: Int, row: Int, pressed: Boolean, moving: Boolean)
    fun scrollBy(delta: Int)
    fun jumpToBottom()
    fun setSelection(selection: TermSelection?)
    fun copySelection()
    fun pasteClipboard()
    fun setFontSize(size: Float)
    fun setExtraKeyRows(rows: Int)
    fun setAgentKeyLayout(agentId: String, layout: AgentKeyLayout)
    fun setFont(kind: TermFontKind)
    fun setScrollback(lines: Int)
    fun onSystemThemeChanged(prefersDark: Boolean)
    fun followSystemTheme()
    fun selectTheme(name: String)
    fun exportAppearance(): String
    fun importAppearance(text: String)
    fun copyAppearance()
    fun importAppearanceFromClipboard()
    fun proposeSize(cols: Int, rows: Int)
    fun requestRename(session: SessionInfo)
    fun onRenameDraft(value: String)
    fun confirmRename()
    fun dismissRename()
    fun killSession(id: Long)
    fun attachedSession(): SessionInfo?
    fun openTabs(): List<SessionInfo>
    fun toggleChat()
    fun loadOlderChat()
    fun connection(id: String): ServerConnection?
    fun reconnect(id: String)
    fun disconnectServer(id: String)
    fun disconnectAll()
    fun onUiForegroundChanged(foreground: Boolean)
    fun setKeepConnectionsInBackground(enabled: Boolean)
}

class ConnectionViewModel(private val controller: ConnectionController) : ViewModel(), ConnectionActions by controller {
    override fun onCleared() {
        controller.releaseTerminalUi()
        super.onCleared()
    }
    companion object {
        val Factory: ViewModelProvider.Factory get() = ConnectionController.Factory
        internal fun significantResize(cols: Int, rows: Int, prevCols: Int, prevRows: Int): Boolean =
            ConnectionController.significantResize(cols, rows, prevCols, prevRows)
    }
}

class ConnectionController(
    private val app: Application,
) : ConnectionActions {
    private val controllerScope = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.SupervisorJob() + Dispatchers.Main.immediate)
    private val connectionPreferences = ai.eclosion.octoterm.android.connection.ConnectionPreferences(app)
    private val store = ConnectionStore(app)
    private val appearanceStore = AppearanceStore(app)
    private var systemDark = app.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK ==
        Configuration.UI_MODE_NIGHT_YES
    private val main = Handler(Looper.getMainLooper())
    private val http = OkHttpClient.Builder().callTimeout(8, TimeUnit.SECONDS).build()
    private var client: OctoClient? = null
    private val links = ai.eclosion.octoterm.android.connection.ServerConnectionRegistry<LiveServerConnection> { link ->
        link.agentJob?.cancel(); link.timeoutJob?.cancel(); link.client.close()
        link.status = ServerConnectionStatus.Disconnected
    }
    private var selectedServerId: String?
        get() = links.selectedId
        set(value) { links.select(value) }
    private var uiForeground = false
    private var viewportsSuspended = false
    private val opens = LinkedHashMap<Long, OpenSession>()
    private var activeId: Long? = null
    private var httpOrigin: String? = null
    private var httpToken: String? = null
    private var awaitingCreated = false
    /** 点通知要去的会话;连上并拿到会话列表后消费,连接失败/断开即作废。 */
    private val pendingOpen = PendingOpenTracker()
    private var chatJob: Job? = null
    private var olderChatJob: Job? = null
    private var chatTimeline = 0L
    private var chatAgain = false
    private var chatCursor: String? = null
    /** 当前消息属于哪个 agent 会话。换了 agent 就得整段重拉,旧游标对新记录没有意义 */
    private var chatAgentKey: String? = null
    @Volatile
    override var painted: TermFrame = TermFrame.Empty
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
            keepConnectionsInBackground = connectionPreferences.keepInBackground,
        ),
    )
    override val state: StateFlow<ConnectionUiState> = _state.asStateFlow()

    // 用 Channel 而不是 replay=0 的 SharedFlow:冷启动点通知时 handleIntent 先于 setContent,
    // 那时还没有收集者,一次性提示会直接丢失;Channel 会缓存到界面开始收集为止。
    // 容量限定为 4 并丢最旧的:突发事件不该堆积成几十条迟到很久才弹出的 snackbar。
    private val _messages = Channel<UserMessage>(capacity = 4, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    override val messages: Flow<UserMessage> = _messages.receiveAsFlow()

    override fun openAddSheet() {
        val clip = clipboardUrl()
        if (clip == null) {
            startEditor(null, null)
        } else {
            _state.update { it.copy(showAddSheet = true, editor = it.editor.copy(clipboardUrl = clip)) }
        }
    }

    override fun dismissAddSheet() {
        _state.update { it.copy(showAddSheet = false) }
    }

    override fun addFromClipboard() {
        val clip = _state.value.editor.clipboardUrl ?: clipboardUrl()
        _state.update { it.copy(showAddSheet = false) }
        startEditor(null, clip)
    }

    override fun addManually() {
        _state.update { it.copy(showAddSheet = false) }
        startEditor(null, null)
    }

    override fun edit(id: String) {
        startEditor(id, null)
    }

    override fun closeEditor() {
        _state.update {
            it.copy(screen = AppScreen.List, editor = EditorState(clipboardUrl = clipboardUrl()))
        }
    }

    override fun openAppearance() {
        _state.update { it.copy(screen = AppScreen.Appearance) }
    }

    override fun closeAppearance() {
        _state.update { it.copy(screen = AppScreen.List) }
    }

    override fun openCloudAccount() {
        _state.update { it.copy(screen = AppScreen.CloudAccount) }
    }

    override fun closeCloudAccount() {
        _state.update { it.copy(screen = AppScreen.List) }
    }

    override fun onNameChange(value: String) {
        _state.update { it.copy(editor = it.editor.copy(name = value)) }
    }

    override fun onUrlChange(value: String) {
        _state.update { it.copy(editor = refreshWarnings(it.editor.copy(url = value, urlError = null))) }
    }

    override fun onTokenChange(value: String) {
        _state.update { it.copy(editor = it.editor.copy(token = value, tokenError = null)) }
    }

    override fun pasteClipboardIntoEditor() {
        val clip = clipboardUrl() ?: return
        applyPrefill(_state.value.editor.copy(clipboardUrl = clip), clip)
    }

    override fun saveEditor() {
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
        if (editingId != null) {
            val previous = current.connections.firstOrNull { it.id == editingId }
            val next = updated.firstOrNull { it.id == editingId }
            if (previous?.url != next?.url || previous?.token != next?.token) disconnectServer(editingId)
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

    override fun requestDelete(connection: ServerConnection) {
        _state.update { it.copy(pendingDelete = connection) }
    }

    override fun dismissDelete() {
        _state.update { it.copy(pendingDelete = null) }
    }

    override fun confirmDelete() {
        val target = _state.value.pendingDelete ?: return
        disconnectServer(target.id)
        links.remove(target.id)
        val updated = _state.value.connections.filterNot { it.id == target.id }
        persist(updated)
        _state.update { it.copy(connections = updated, pendingDelete = null) }
        publishConnections()
        _messages.trySend(UserMessage.Deleted)
    }

    override fun connect(id: String) {
        val connection = connection(id) ?: run { pendingOpen.onConnectFailed(); return }
        if (selectedServerId != null && selectedServerId != id) leaveServer()
        pendingOpen.onConnectStart(id)
        selectedServerId = id
        var link = links[id]
        if (link == null || !link.status.active || link.connection.url != connection.url || link.connection.token != connection.token) {
            link = createLink(connection) ?: run { leaveServer(); return }
        }
        activateLink(link)
        if (link.status == ServerConnectionStatus.Connected) {
            link.client.send(ClientMsg.ListSessions)
            refreshAgents(link)
        }
    }

    private fun activateLink(link: LiveServerConnection) {
        selectedServerId = link.connection.id
        client = link.client
        httpOrigin = link.endpoint.httpOrigin()
        httpToken = link.endpoint.token
        val connected = link.status == ServerConnectionStatus.Connected
        _state.update { it.copy(
            screen = if (connected || link.sessions.isNotEmpty() || it.screen is AppScreen.Server) AppScreen.Server(link.connection.id) else AppScreen.List,
            connectingId = if (connected) null else link.connection.id,
            sessions = link.sessions, agents = link.agents, transcriptEnabled = link.transcriptEnabled,
            serverOs = link.os, reconnecting = !connected,
        ) }
    }

    private fun createLink(connection: ServerConnection): LiveServerConnection? {
        val endpoint = try { ConnectionUrl.parse(connection.url, connection.token) }
        catch (_: ConnectionUrlException) {
            pendingOpen.onConnectFailed(); _messages.trySend(UserMessage.Unexpected); return null
        }
        if (endpoint.token.isNullOrBlank()) {
            pendingOpen.onConnectFailed(); _messages.trySend(UserMessage.AuthFailed(null)); return null
        }
        val next = OctoClient()
        val link = LiveServerConnection(connection, endpoint, next)
        links[connection.id] = link
        if (selectedServerId == connection.id) {
            client = next; httpOrigin = endpoint.httpOrigin(); httpToken = endpoint.token
        }
        publishConnections()
        next.onOpen = { onMain {
            if (!links.isCurrent(connection.id, link)) return@onMain
            link.status = ServerConnectionStatus.Connected
            link.timeoutJob?.cancel()
            val now = System.currentTimeMillis()
            val updated = _state.value.connections.map { if (it.id == connection.id) it.copy(lastUsedAt = now) else it }
            persist(updated)
            _state.update { it.copy(connections = updated) }
            publishConnections()
            next.send(ClientMsg.ListSessions)
            link.agentRevision++
            refreshAgents(link)
            if (client === next && selectedServerId == connection.id) {
                _state.update { it.copy(connectingId = null, reconnecting = false, screen = AppScreen.Server(connection.id)) }
                if (viewportsSuspended && uiForeground) resumeViewports()
            }
        } }
        next.onReconnecting = { onMain {
            if (!links.isCurrent(connection.id, link)) return@onMain
            link.status = ServerConnectionStatus.Reconnecting
            publishConnections()
            if (client === next) _state.update { it.copy(reconnecting = true) }
        } }
        next.onFatal = { message -> onMain {
            if (!links.isCurrent(connection.id, link)) return@onMain
            links.disconnect(connection.id)
            link.status = ServerConnectionStatus.Failed
            if (client === next) {
                pendingOpen.onConnectFailed()
                leaveServer()
                _messages.trySend(UserMessage.AuthFailed(message))
            }
            publishConnections()
        } }
        next.onControl = { msg -> onMain {
            if (!links.isCurrent(connection.id, link)) return@onMain
            when (msg) {
                is ServerMsg.HelloOk -> {
                    link.os = msg.os
                    CloudLink.record(_state.value.connections, connection.id, msg.cloudServerId)?.let { updated ->
                        persist(updated); _state.update { it.copy(connections = updated) }
                    }
                }
                is ServerMsg.Sessions -> link.sessions = msg.sessions
                is ServerMsg.SessionEvent -> next.send(ClientMsg.ListSessions)
                is ServerMsg.AgentEvent -> {
                    link.agentRevision++
                    link.agents = AgentSessions.applyEvent(link.agents, msg.session)
                    if (msg.session.state != AgentState.Done && link.agents[msg.session.key]?.supportsTranscript == null) refreshAgents(link)
                }
                else -> Unit
            }
            if (client === next && selectedServerId == connection.id) {
                _state.update { it.copy(serverOs = link.os, agents = link.agents, transcriptEnabled = link.transcriptEnabled) }
                handleServer(msg)
            }
        } }
        next.onChannelData = { channel, payload -> onMain {
            if (client !== next || selectedServerId != connection.id || viewportsSuspended) return@onMain
            val open = opens.values.firstOrNull { it.channel == channel } ?: return@onMain
            open.emulator.write(payload)
            next.noteData(channel, payload.size)
            if (open.id == activeId && open.paintGate.shouldPublishWrite()) schedulePublish()
        } }
        next.connect(endpoint.webSocketUrl, endpoint.token!!)
        link.timeoutJob = controllerScope.launch {
            kotlinx.coroutines.delay(12_000)
            if (links.isCurrent(connection.id, link) && link.status == ServerConnectionStatus.Connecting) {
                links.disconnect(connection.id); link.status = ServerConnectionStatus.Failed
                if (client === next) { pendingOpen.onConnectFailed(); leaveServer(); _messages.trySend(UserMessage.Timeout) }
                publishConnections()
            }
        }
        return link
    }

    /** Navigation releases terminal viewports but leaves the server transport application-owned. */
    override fun leaveServer() {
        pendingOpen.onDisconnect()
        leaveTerminal()
        client = null
        selectedServerId = null
        httpOrigin = null; httpToken = null
        _state.update { it.copy(
            screen = AppScreen.List, sessions = emptyList(), agents = emptyMap(), connectingId = null,
            attachedId = null, openSessionIds = emptyList(), reconnecting = false, serverOs = "",
            transcriptEnabled = false, chat = ChatUiState(), pendingRename = null,
            launcherMenu = LauncherMenuState(), selection = null, scrollOffset = 0, mouseTracking = false,
        ) }
    }

    override fun disconnectServer(id: String) {
        if (selectedServerId == id) leaveServer()
        links.disconnect(id)
        publishConnections()
    }

    override fun reconnect(id: String) {
        val wasSelected = selectedServerId == id
        disconnectServer(id)
        val connection = connection(id) ?: return
        if (wasSelected) connect(id) else createLink(connection)
    }

    override fun disconnectAll() {
        leaveServer()
        links.disconnectAll()
        connectionPreferences.desiredConnections = emptySet()
        publishConnections()
    }

    private fun publishConnections(persistDesired: Boolean = true) {
        val statuses = links.mapValues { it.value.status }
        _state.update { it.copy(connectionStatuses = statuses) }
        if (persistDesired) connectionPreferences.desiredConnections = links.activeIds
        syncConnectionService()
    }

    private fun syncConnectionService() {
        val needed = _state.value.keepConnectionsInBackground && links.values.any { it.status.active }
        if (!needed) ai.eclosion.octoterm.android.connection.ConnectionService.stop(app)
        else if (uiForeground) {
            if (!ai.eclosion.octoterm.android.connection.ConnectionService.start(app)) {
                _messages.trySend(UserMessage.BackgroundUnavailable)
            }
        }
    }

    fun restoreDesiredConnections() {
        val desired = connectionPreferences.desiredConnections
        for (connection in _state.value.connections) {
            if (connection.id !in desired || links[connection.id]?.status?.active == true) continue
            val link = createLink(connection) ?: continue
            if (selectedServerId == connection.id) activateLink(link)
        }
    }

    fun onConnectionServiceRunning(running: Boolean) {
        _state.update { it.copy(connectionServiceRunning = running) }
        if (!running) syncConnectionService()
    }

    override fun setKeepConnectionsInBackground(enabled: Boolean) {
        connectionPreferences.keepInBackground = enabled
        _state.update { it.copy(keepConnectionsInBackground = enabled) }
        syncConnectionService()
    }

    override fun onUiForegroundChanged(foreground: Boolean) {
        uiForeground = foreground
        if (foreground) {
            restoreDesiredConnections()
            syncConnectionService()
            if (viewportsSuspended && client != null && !_state.value.reconnecting) resumeViewports()
            if (_state.value.chatOpen) refreshChat(incremental = true)
        } else {
            suspendViewports()
            chatJob?.cancel(); chatJob = null
            olderChatJob?.cancel(); olderChatJob = null
            _state.update { it.copy(chat = it.chat.copy(olderLoading = false)) }
            if (!_state.value.keepConnectionsInBackground) {
                val desired = connectionPreferences.desiredConnections
                links.disconnectAll()
                selectedServerId = (_state.value.screen as? AppScreen.Server)?.connectionId
                client = null
                _state.update { it.copy(reconnecting = selectedServerId != null) }
                publishConnections(persistDesired = false)
                connectionPreferences.desiredConnections = desired
            }
        }
    }

    private fun suspendViewports() {
        if (viewportsSuspended || opens.isEmpty()) return
        main.removeCallbacks(proposeFlush)
        main.removeCallbacks(publishSoon)
        for (open in opens.values) {
            client?.detach(open.channel); open.paintGate.onDetached(); open.pendingAttach = false
        }
        viewportsSuspended = true
    }

    private fun resumeViewports() {
        if (!uiForeground || activeId == null || _state.value.chatOpen) return
        viewportsSuspended = false
        for (open in opens.values) { open.pendingAttach = true; open.paintGate.onSessionOpen() }
        main.removeCallbacks(proposeFlush); main.post(proposeFlush)
    }

    override fun openFromNotification(cloudServerId: String, session: Long, agentId: String?) {
        val connection = CloudLink.find(_state.value.connections, cloudServerId) ?: run {
            _messages.trySend(UserMessage.ConnectServerFirst); return
        }
        val target = PendingOpen(connection.id, session, agentId?.let { AgentPush.tag(cloudServerId, session, it) })
        if (selectedServerId != connection.id) leaveServer()
        pendingOpen.onTap(target)
        connect(connection.id)
        client?.send(ClientMsg.ListSessions)
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

    override fun openLauncherMenu() {
        val origin = httpOrigin
        val token = httpToken
        if (origin.isNullOrEmpty() || token.isNullOrEmpty()) return
        _state.update {
            it.copy(launcherMenu = LauncherMenuState(open = true, loading = true, items = listOf(Launchers.fallback())))
        }
        controllerScope.launch {
            val items = withContext(Dispatchers.IO) { Launchers.fetch(http, origin, token) }
            if (!_state.value.launcherMenu.open) return@launch
            _state.update { it.copy(launcherMenu = LauncherMenuState(open = true, loading = false, items = items)) }
        }
    }

    override fun dismissLauncherMenu() {
        _state.update { it.copy(launcherMenu = LauncherMenuState()) }
    }

    override fun newSession(launcher: Launcher) {
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

    override suspend fun loadHistory(query: String, agent: String, offset: Int, refresh: Boolean, workspace: String): HistoryPage {
        val origin = httpOrigin ?: throw HistoryFailure("unreadable")
        val token = httpToken ?: throw HistoryFailure("unreadable")
        return withContext(Dispatchers.IO) { AgentHistory.fetch(http, origin, token, query, agent, offset, refresh, workspace) }
    }

    override suspend fun previewHistory(entry: HistoryEntry): ChatWindow {
        val origin = httpOrigin ?: throw HistoryFailure("unreadable")
        val token = httpToken ?: throw HistoryFailure("unreadable")
        return withContext(Dispatchers.IO) { AgentHistory.messages(http, origin, token, entry) }
    }

    override suspend fun resumeHistory(entry: HistoryEntry) {
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
        selectedServerId?.let { links[it]?.agentRevision = (links[it]?.agentRevision ?: 0) + 1 }
        refreshAgents()
    }

    override fun openSession(sessionId: Long) {
        if (_state.value.chat.sessionId != sessionId) resetChat()
        val existing = opens[sessionId]
        if (existing != null) {
            activeId = sessionId
            _state.update { it.copy(selection = null) }
            if (viewportsSuspended && uiForeground) resumeViewports()
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
        if (!uiForeground || _state.value.chatOpen) viewportsSuspended = true
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

    /** Returning to the session list releases every local terminal viewport. */
    override fun leaveTerminal() {
        resetChat()
        main.removeCallbacks(proposeFlush)
        main.removeCallbacks(publishSoon)
        closeAllSessions()
        viewportsSuspended = false
        _state.update { it.copy(attachedId = null, openSessionIds = emptyList(), selection = null, scrollOffset = 0, mouseTracking = false) }
    }

    override fun closeOpenSession(id: Long) {
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

    override fun activeEmulator(): VtEmulator? = opens[activeId]?.emulator

    override fun sendInput(bytes: ByteArray) {
        sendInput(activeId ?: return, bytes)
    }

    override fun sendInput(sessionId: Long, bytes: ByteArray) {
        if (_state.value.reconnecting || bytes.isEmpty()) return
        val open = opens[sessionId] ?: return
        open.emulator.scrollToBottom()
        client?.sendInput(open.channel, bytes)
        _state.update { it.copy(selection = null) }
        publishTerm()
    }

    override fun sendMouse(col: Int, row: Int, pressed: Boolean, moving: Boolean) {
        val open = opens[activeId] ?: return
        if (open.emulator.scrollOffset != 0) open.emulator.scrollToBottom()
        val bytes = open.emulator.encodePointer(col, row, button = 0, pressed = pressed, moving = moving) ?: return
        if (bytes.isEmpty()) return
        client?.sendInput(open.channel, bytes)
    }

    override fun scrollBy(delta: Int) {
        val open = opens[activeId] ?: return
        open.emulator.scrollBy(delta)
        _state.update { it.copy(selection = null) }
        publishTerm(allowEmpty = true)
    }

    override fun jumpToBottom() {
        val open = opens[activeId] ?: return
        open.emulator.scrollToBottom()
        publishTerm(allowEmpty = true)
    }

    override fun setSelection(selection: TermSelection?) {
        _state.update { it.copy(selection = selection) }
    }

    override fun copySelection() {
        val open = opens[activeId] ?: return
        val selection = _state.value.selection ?: return
        val text = open.emulator.copyViewport(selection.x0, selection.y0, selection.x1, selection.y1)
        if (text.isEmpty()) return
        val clipboard = app.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("octoterm", text))
        _messages.trySend(UserMessage.Copied)
    }

    override fun pasteClipboard() {
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

    override fun setFontSize(size: Float) = updateAppearance(_state.value.appearance.withFontSize(size))

    override fun setExtraKeyRows(rows: Int) {
        val next = rows.coerceIn(1, 2)
        appearanceStore.saveExtraKeyRows(next)
        _state.update { it.copy(extraKeyRows = next) }
    }

    override fun setAgentKeyLayout(agentId: String, layout: AgentKeyLayout) {
        val next = _state.value.agentKeyLayouts.toMutableMap()
        if (layout == AgentKeyLayout()) next.remove(agentId) else next[agentId] = layout
        appearanceStore.saveAgentKeyLayouts(next)
        _state.update { it.copy(agentKeyLayouts = next) }
    }

    override fun setFont(kind: TermFontKind) = updateAppearance(_state.value.appearance.withFont(kind))

    override fun setScrollback(lines: Int) = updateAppearance(_state.value.appearance.withScrollback(lines))

    override fun onSystemThemeChanged(prefersDark: Boolean) {
        systemDark = prefersDark
        _state.update {
            if (it.appearanceFollowsSystem) {
                it.copy(appearance = it.appearance.withTheme(AppearanceCodec.default(prefersDark)))
            } else it
        }
        reportColors()
    }

    override fun followSystemTheme() {
        updateAppearance(
            _state.value.appearance.withTheme(AppearanceCodec.default(systemDark)),
            followsSystem = true,
        )
    }

    override fun selectTheme(name: String) {
        val theme = AppearanceCodec.builtin(name) ?: return
        val current = _state.value.appearance
        updateAppearance(
            current.withTheme(theme),
            followsSystem = false,
        )
    }

    override fun exportAppearance(): String = AppearanceCodec.exportJson(_state.value.appearance)

    override fun importAppearance(text: String) {
        val next = AppearanceCodec.importJson(text, _state.value.appearance)
        if (next == null) {
            _messages.trySend(UserMessage.ImportFailed)
            return
        }
        updateAppearance(next, followsSystem = false)
        _messages.trySend(UserMessage.Saved)
    }

    override fun copyAppearance() {
        val clipboard = app.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("octoterm-appearance", exportAppearance()))
        _messages.trySend(UserMessage.Copied)
    }

    override fun importAppearanceFromClipboard() {
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

    override fun proposeSize(cols: Int, rows: Int) {
        proposedCols = cols.coerceAtLeast(20)
        proposedRows = rows.coerceAtLeast(5)
        main.removeCallbacks(proposeFlush)
        main.postDelayed(proposeFlush, PROPOSE_DEBOUNCE_MS)
    }

    override fun requestRename(session: SessionInfo) {
        _state.update { it.copy(pendingRename = session, renameDraft = session.name) }
    }

    override fun onRenameDraft(value: String) {
        _state.update { it.copy(renameDraft = value) }
    }

    override fun confirmRename() {
        val target = _state.value.pendingRename ?: return
        val name = _state.value.renameDraft.trim()
        if (name.isNotEmpty()) {
            client?.send(ClientMsg.RenameSession(target.id, name))
        }
        _state.update { it.copy(pendingRename = null, renameDraft = "") }
    }

    override fun dismissRename() {
        _state.update { it.copy(pendingRename = null, renameDraft = "") }
    }

    override fun killSession(id: Long) {
        if (opens.containsKey(id)) closeOpenSession(id)
        client?.send(ClientMsg.KillSession(id))
    }

    override fun attachedSession(): SessionInfo? {
        val id = _state.value.attachedId ?: return null
        return _state.value.sessions.firstOrNull { it.id == id }
    }

    override fun openTabs(): List<SessionInfo> {
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
    override fun toggleChat() {
        val st = _state.value
        if (st.chatOpen) {
            resetChat()
            resumeViewports()
            return
        }
        val sessionId = st.attachedId ?: return
        if (!st.chatAvailable) return
        suspendViewports()
        _state.update { it.copy(chat = ChatUiState(sessionId = sessionId, loading = true)) }
        // 进聊天视图才去拉 —— 不看的时候不该白读别人的文件
        refreshChat(incremental = false)
    }

    private fun onAgentEvent(ev: AgentSession) {
        closeChatIfUnavailable()
        if (uiForeground && _state.value.chatOpen && ev.session == _state.value.chat.sessionId) refreshChat(incremental = true)
    }

    private fun refreshAgents() { selectedServerId?.let { links[it] }?.let(::refreshAgents) }

    private fun refreshAgents(link: LiveServerConnection) {
        if (link.agentJob?.isActive == true) { link.agentAgain = true; return }
        link.agentJob = controllerScope.launch {
            do {
                link.agentAgain = false
                val revision = link.agentRevision
                val snapshot = withContext(Dispatchers.IO) { AgentSessions.fetch(http, link.endpoint.httpOrigin(), link.endpoint.token.orEmpty()) }
                if (!links.isCurrent(link.connection.id, link)) return@launch
                if (revision != link.agentRevision) { link.agentAgain = true; continue }
                link.agents = AgentSessions.replaceAll(snapshot.sessions)
                link.transcriptEnabled = snapshot.transcriptEnabled
                if (client === link.client && selectedServerId == link.connection.id) {
                    _state.update { it.copy(agents = link.agents, transcriptEnabled = snapshot.transcriptEnabled) }
                    closeChatIfUnavailable()
                    if (uiForeground && _state.value.chatOpen) refreshChat(incremental = true)
                }
            } while (link.agentAgain)
        }
    }

    private fun closeChatIfUnavailable() {
        val st = _state.value
        if (st.chat.sessionId != null && !st.chatAvailable) { resetChat(); resumeViewports() }
    }

    private fun resetChat() {
        chatTimeline++
        olderChatJob?.cancel()
        olderChatJob = null
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
        chatJob = controllerScope.launch {
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
                        if (win.reset || !inc) {
                            chatTimeline++
                            olderChatJob?.cancel(); olderChatJob = null
                        }
                        chatCursor = win.cursor
                        chatAgentKey = agent.key
                        _state.update {
                            it.copy(
                                chat = it.chat.copy(
                                    messages = Chat.merge(if (inc) it.chat.messages else emptyList(), win),
                                    notice = null,
                                    loading = false,
                                    olderCursor = if (win.reset || !inc) win.olderCursor else it.chat.olderCursor,
                                    historySupported = if (win.reset || !inc) win.historySupported else it.chat.historySupported,
                                    olderLoading = if (win.reset || !inc) false else it.chat.olderLoading,
                                    olderFailed = if (win.reset || !inc) false else it.chat.olderFailed,
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

    override fun loadOlderChat() {
        val st = _state.value
        val sessionId = st.chat.sessionId ?: return
        val before = st.chat.olderCursor ?: return
        val agent = AgentSessions.chatAgentFor(st.agents, sessionId) ?: return
        val origin = httpOrigin ?: return
        val token = httpToken ?: return
        if (!st.chatOpen || st.chat.olderLoading || olderChatJob?.isActive == true) return
        val timeline = chatTimeline
        _state.update { it.copy(chat = it.chat.copy(olderLoading = true, olderFailed = false)) }
        olderChatJob = controllerScope.launch {
            val win = withContext(Dispatchers.IO) {
                Chat.fetch(http, origin, token, agent.agentId, agent.agentSessionId, null, before)
            }
            val current = _state.value
            if (timeline != chatTimeline || !current.chatOpen || current.chat.sessionId != sessionId ||
                current.chat.olderCursor != before || AgentSessions.chatAgentFor(current.agents, sessionId)?.key != agent.key) return@launch
            if (win !is ChatWindow.Transcript || (!win.reset && win.olderCursor == before)) {
                _state.update { it.copy(chat = it.chat.copy(olderLoading = false, olderFailed = true)) }
                return@launch
            }
            if (win.reset) {
                chatJob?.cancel(); chatJob = null; chatAgain = false
                chatTimeline++
                chatCursor = win.cursor
                chatAgentKey = agent.key
            }
            _state.update { it.copy(chat = it.chat.copy(
                messages = Chat.merge(it.chat.messages, win, prepend = true),
                olderCursor = win.olderCursor, historySupported = win.historySupported,
                olderLoading = false, olderFailed = false, historyPage = it.chat.historyPage + 1,
            )) }
        }
    }

    private fun applyProposedSize() {
        if (viewportsSuspended || !uiForeground || _state.value.chatOpen) return
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

    override fun connection(id: String): ServerConnection? = _state.value.connections.firstOrNull { it.id == id }

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

    fun releaseTerminalUi() {
        main.removeCallbacks(proposeFlush)
        main.removeCallbacks(publishSoon)
        leaveTerminal()
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
                ConnectionViewModel((app as ai.eclosion.octoterm.android.OctotermApplication).connections)
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

private class LiveServerConnection(
    val connection: ServerConnection,
    val endpoint: ParsedEndpoint,
    val client: OctoClient,
) {
    var status = ServerConnectionStatus.Connecting
    var sessions: List<SessionInfo> = emptyList()
    var agents: Map<String, AgentSession> = emptyMap()
    var transcriptEnabled = false
    var os = ""
    var agentJob: Job? = null
    var agentAgain = false
    var agentRevision = 0L
    var timeoutJob: Job? = null
}
