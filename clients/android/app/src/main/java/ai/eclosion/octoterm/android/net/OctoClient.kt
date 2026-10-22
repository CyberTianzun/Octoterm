package ai.eclosion.octoterm.android.net

import ai.eclosion.octoterm.android.wire.AttachMode
import ai.eclosion.octoterm.android.wire.ClientMsg
import ai.eclosion.octoterm.android.wire.ControlJson
import ai.eclosion.octoterm.android.wire.Frame
import ai.eclosion.octoterm.android.wire.ServerMsg
import ai.eclosion.octoterm.android.wire.TermColors
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import java.util.concurrent.TimeUnit

class OctoClient(
    private val http: WebSocket.Factory = defaultClient(),
    private val schedule: (Long, () -> Unit) -> Unit = { delay, block ->
        android.os.Handler(android.os.Looper.getMainLooper()).postDelayed(block, delay)
    },
) {
    private data class AttachState(
        val sessionId: Long,
        var cols: Int,
        var rows: Int,
        var lastSeq: Long?,
        var colors: TermColors,
    )

    var onOpen: () -> Unit = {}
    var onControl: (ServerMsg) -> Unit = {}
    var onChannelData: (channel: Int, payload: ByteArray) -> Unit = { _, _ -> }
    var onReconnecting: () -> Unit = {}
    var onFatal: (String) -> Unit = {}

    private var url = ""
    private var token = ""
    private var ws: WebSocket? = null
    private val attachments = LinkedHashMap<Int, AttachState>()
    // Public actions and socket callbacks run on the main thread, including reconnects.
    private var closedByUser = false
    private var authOk = false
    private var preHelloError: String? = null
    private var generation = 0L
    private var backoffMs = 250L

    fun connect(webSocketUrl: String, token: String) {
        close()
        this.url = webSocketUrl
        this.token = token
        closedByUser = false
        backoffMs = 250L
        dial()
    }

    fun close() {
        closedByUser = true
        authOk = false
        generation++
        ws?.close(1000, null)
        ws = null
        attachments.clear()
    }

    fun send(msg: ClientMsg) {
        if (authOk) ws?.send(ByteString.of(*ControlJson.encode(msg)))
    }

    fun sendInput(channel: Int, bytes: ByteArray) {
        if (bytes.isEmpty() || !authOk || channel !in attachments) return
        ws?.send(ByteString.of(*Frame.encode(channel, bytes)))
    }

    fun attach(sessionId: Long, channel: Int, cols: Int, rows: Int, colors: TermColors) {
        attachments[channel] = AttachState(sessionId, cols, rows, null, colors)
        send(ClientMsg.Attach(sessionId, channel, null, cols, rows, colors))
    }

    fun detach(channel: Int) {
        if (attachments.remove(channel) == null) return
        send(ClientMsg.Detach(channel))
    }

    fun resize(channel: Int, cols: Int, rows: Int) {
        val state = attachments[channel] ?: return
        if (state.cols == cols && state.rows == rows) return
        state.cols = cols
        state.rows = rows
        send(ClientMsg.Resize(channel, cols, rows))
    }

    /** 换了主题。协议没有单独的改颜色消息(新增 client→server 类型要 bump proto),
     *  按 X4 搭在 resize 上,尺寸照旧。 */
    fun setColors(channel: Int, colors: TermColors) {
        val state = attachments[channel] ?: return
        if (state.colors == colors) return
        state.colors = colors
        send(ClientMsg.Resize(channel, state.cols, state.rows, colors))
    }

    fun noteData(channel: Int, byteLen: Int) {
        val state = attachments[channel] ?: return
        val seq = state.lastSeq ?: return
        state.lastSeq = seq + byteLen
    }

    private fun dial() {
        val attempt = ++generation
        authOk = false
        preHelloError = null
        fun dispatch(block: () -> Unit) = schedule(0) {
            if (!closedByUser && generation == attempt) block()
        }
        val request = Request.Builder().url(url).build()
        ws = http.newWebSocket(
            request,
            object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: Response) {
                    dispatch { webSocket.send(ByteString.of(*ControlJson.encodeHello(token))) }
                }

                override fun onMessage(webSocket: WebSocket, text: String) {
                    // T4: text frames are ignored after handshake; reject during hello.
                }

                override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                    dispatch { handle(bytes.toByteArray()) }
                }

                override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                    dispatch {
                        if (response?.code == 401 || response?.code == 403) preHelloError = "HTTP ${response.code}"
                        onSocketGone()
                    }
                }

                override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                    dispatch { onSocketGone() }
                }

                override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                    dispatch { webSocket.close(code, reason); onSocketGone() }
                }
            },
        )
    }

    private fun handle(data: ByteArray) {
        val frame = try {
            Frame.decode(data)
        } catch (_: Exception) {
            return
        }
        if (frame.channel != Frame.CONTROL_CHANNEL) {
            onChannelData(frame.channel, frame.payload)
            return
        }
        val msg = try {
            ControlJson.parse(frame.payload)
        } catch (_: Exception) {
            return
        }
        when (msg) {
            is ServerMsg.Error -> {
                if (!authOk) preHelloError = msg.message
            }
            is ServerMsg.HelloOk -> {
                authOk = true
                backoffMs = 250L
                reattachAll()
                onOpen()
            }
            is ServerMsg.ResyncEnd -> {
                val state = attachments[msg.channel]
                if (state != null) state.lastSeq = msg.seq
            }
            is ServerMsg.Attached -> {
                val state = attachments[msg.channel]
                if (state != null && msg.mode == AttachMode.Replay) {
                    // lastSeq already set to the resume point we sent; do not use attached.seq
                }
            }
            else -> Unit
        }
        onControl(msg)
    }

    private fun reattachAll() {
        for ((channel, state) in attachments) {
            send(
                ClientMsg.Attach(
                    id = state.sessionId,
                    channel = channel,
                    lastSeq = state.lastSeq,
                    cols = state.cols,
                    rows = state.rows,
                    colors = state.colors,
                ),
            )
        }
    }

    private fun onSocketGone() {
        if (closedByUser) return
        // Retire this socket before callbacks can close/reconnect it. Duplicate and late
        // callbacks cannot schedule a second dial or update a replacement connection.
        val retired = ++generation
        ws?.cancel()
        ws = null
        if (!authOk && preHelloError != null) {
            closedByUser = true
            onFatal(preHelloError!!)
            return
        }
        onReconnecting()
        val delay = backoffMs
        backoffMs = (backoffMs * 2).coerceAtMost(10_000L)
        schedule(delay) {
            if (!closedByUser && generation == retired) dial()
        }
    }

    companion object {
        private fun defaultClient(): OkHttpClient {
            return OkHttpClient.Builder()
                .connectTimeout(8, TimeUnit.SECONDS)
                .readTimeout(0, TimeUnit.MILLISECONDS)
                .writeTimeout(0, TimeUnit.MILLISECONDS)
                .pingInterval(30, TimeUnit.SECONDS)
                .retryOnConnectionFailure(false)
                .build()
        }
    }
}
