package ai.eclosion.octoterm.android.net

import ai.eclosion.octoterm.android.wire.AttachMode
import ai.eclosion.octoterm.android.wire.ClientMsg
import ai.eclosion.octoterm.android.wire.ControlJson
import ai.eclosion.octoterm.android.wire.Frame
import ai.eclosion.octoterm.android.wire.ServerMsg
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

class OctoClient(
    private val http: OkHttpClient = defaultClient(),
) {
    private data class AttachState(
        val sessionId: Long,
        var cols: Int,
        var rows: Int,
        var lastSeq: Long?,
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
    private val closedByUser = AtomicBoolean(false)
    private val authOk = AtomicBoolean(false)
    @Volatile private var preHelloError: String? = null
    private var backoffMs = 250L

    fun connect(webSocketUrl: String, token: String) {
        this.url = webSocketUrl
        this.token = token
        closedByUser.set(false)
        backoffMs = 250L
        dial()
    }

    fun close() {
        closedByUser.set(true)
        ws?.close(1000, null)
        ws = null
        attachments.clear()
    }

    fun send(msg: ClientMsg) {
        ws?.send(ByteString.of(*ControlJson.encode(msg)))
    }

    fun sendInput(channel: Int, bytes: ByteArray) {
        if (bytes.isEmpty()) return
        ws?.send(ByteString.of(*Frame.encode(channel, bytes)))
    }

    fun attach(sessionId: Long, channel: Int, cols: Int, rows: Int) {
        attachments[channel] = AttachState(sessionId, cols, rows, null)
        send(ClientMsg.Attach(sessionId, channel, null, cols, rows))
    }

    fun detach(channel: Int) {
        attachments.remove(channel)
        send(ClientMsg.Detach(channel))
    }

    fun resize(channel: Int, cols: Int, rows: Int) {
        val state = attachments[channel] ?: return
        if (state.cols == cols && state.rows == rows) return
        state.cols = cols
        state.rows = rows
        send(ClientMsg.Resize(channel, cols, rows))
    }

    fun noteData(channel: Int, byteLen: Int) {
        val state = attachments[channel] ?: return
        val seq = state.lastSeq ?: return
        state.lastSeq = seq + byteLen
    }

    private fun dial() {
        authOk.set(false)
        preHelloError = null
        val request = Request.Builder().url(url).build()
        ws = http.newWebSocket(
            request,
            object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: Response) {
                    webSocket.send(ByteString.of(*ControlJson.encodeHello(token)))
                }

                override fun onMessage(webSocket: WebSocket, text: String) {
                    // T4: text frames are ignored after handshake; reject during hello.
                }

                override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                    handle(bytes.toByteArray())
                }

                override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                    onSocketGone()
                }

                override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                    onSocketGone()
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
                if (!authOk.get()) preHelloError = msg.message
            }
            is ServerMsg.HelloOk -> {
                authOk.set(true)
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
                ),
            )
        }
    }

    private fun onSocketGone() {
        if (closedByUser.get()) return
        if (!authOk.get() && preHelloError != null) {
            closedByUser.set(true)
            onFatal(preHelloError!!)
            return
        }
        onReconnecting()
        val delay = backoffMs
        backoffMs = (backoffMs * 2).coerceAtMost(10_000L)
        android.os.Handler(android.os.Looper.getMainLooper()).postDelayed({
            if (!closedByUser.get()) dial()
        }, delay)
    }

    companion object {
        private fun defaultClient(): OkHttpClient {
            return OkHttpClient.Builder()
                .connectTimeout(8, TimeUnit.SECONDS)
                .readTimeout(0, TimeUnit.MILLISECONDS)
                .writeTimeout(0, TimeUnit.MILLISECONDS)
                .pingInterval(0, TimeUnit.SECONDS)
                .retryOnConnectionFailure(false)
                .build()
        }
    }
}
