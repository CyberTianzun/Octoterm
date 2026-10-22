package ai.eclosion.octoterm.android.net

import ai.eclosion.octoterm.android.wire.Frame
import ai.eclosion.octoterm.android.wire.TermColors
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class OctoClientLifecycleTest {
    private class Socket(private val request: Request, val listener: WebSocketListener) : WebSocket {
        val sent = mutableListOf<ByteString>()
        var closes = 0
        var cancels = 0
        override fun request() = request
        override fun queueSize() = 0L
        override fun send(text: String) = error("binary protocol only")
        override fun send(bytes: ByteString): Boolean { sent += bytes; return true }
        override fun close(code: Int, reason: String?): Boolean { closes++; return true }
        override fun cancel() { cancels++ }
        fun hello() = listener.onMessage(this, ByteString.of(*Frame.encode(0, """{"type":"hello-ok","proto":1}""".toByteArray())))
        fun fail() = listener.onFailure(this, IllegalStateException("network lost"), null)
        fun types() = sent.map { JSONObject(String(Frame.decode(it.toByteArray()).payload)).getString("type") }
    }

    private class Harness : WebSocket.Factory {
        data class Task(val at: Long, val run: () -> Unit)
        var now = 0L
        val tasks = mutableListOf<Task>()
        val sockets = mutableListOf<Socket>()
        val client = OctoClient(this) { delay, run -> tasks += Task(now + delay, run) }
        override fun newWebSocket(request: Request, listener: WebSocketListener): WebSocket =
            Socket(request, listener).also { sockets += it }
        fun flush() {
            while (true) {
                val task = tasks.firstOrNull { it.at <= now } ?: break
                tasks.remove(task); task.run()
            }
        }
        fun advance(delay: Long) { now += delay; flush() }
        fun connect(): Socket {
            client.connect("ws://localhost/ws", "test-token")
            return sockets.last().also { it.hello(); flush() }
        }
    }

    private val colors = TermColors("#ffffff", "#000000")

    @Test fun leavingTerminalDetachesWithoutClosingServerTransport() {
        val h = Harness(); val socket = h.connect()
        h.client.attach(9, 1, 40, 20, colors)
        h.client.detach(1)
        assertEquals(listOf("attach", "detach"), socket.types())
        assertEquals(0, socket.closes)
        assertEquals(0, socket.cancels)
    }

    @Test fun detachedTerminalIsNotReattachedAfterNetworkReconnect() {
        val h = Harness(); val socket = h.connect()
        h.client.attach(9, 1, 40, 20, colors)
        h.client.detach(1)
        socket.fail(); h.flush(); h.advance(250)
        h.sockets.last().hello(); h.flush()
        assertEquals(2, h.sockets.size)
        assertFalse(h.sockets.last().types().contains("attach"))
    }

    @Test fun repeatedDetachAndDetachBeforeFirstPaintDoNotSendUnknownChannels() {
        val h = Harness(); val socket = h.connect()
        h.client.detach(2)
        h.client.attach(9, 1, 40, 20, colors)
        h.client.detach(1); h.client.detach(1)
        assertEquals(listOf("attach", "detach"), socket.types())
    }

    @Test fun terminalOpenedDuringReconnectWaitsForAuthentication() {
        val h = Harness(); h.client.connect("ws://localhost/ws", "test-token")
        h.client.attach(9, 1, 40, 20, colors)
        assertTrue(h.sockets.last().sent.isEmpty())
        h.sockets.last().hello(); h.flush()
        assertEquals(listOf("attach"), h.sockets.last().types())
    }

    @Test fun retainedTerminalReattachesWithLatestSizeAndReplayCursor() {
        val h = Harness(); val socket = h.connect()
        h.client.attach(9, 1, 40, 20, colors)
        socket.listener.onMessage(socket, ByteString.of(*Frame.encode(0,
            """{"type":"resync-end","channel":1,"seq":10}""".toByteArray())))
        h.flush(); h.client.noteData(1, 5); h.client.resize(1, 50, 25)
        socket.fail(); h.flush(); h.advance(250)
        val replacement = h.sockets.last(); replacement.hello(); h.flush()
        val attach = JSONObject(String(Frame.decode(replacement.sent.single().toByteArray()).payload))
        assertEquals("attach", attach.getString("type"))
        assertEquals(15L, attach.getLong("last_seq"))
        assertEquals(50, attach.getInt("cols"))
        assertEquals(25, attach.getInt("rows"))
    }

    @Test fun duplicateSocketCallbacksOnlyScheduleOneReconnect() {
        val h = Harness(); val socket = h.connect()
        var retries = 0; h.client.onReconnecting = { retries++ }
        socket.fail(); socket.listener.onClosed(socket, 1001, "gone")
        h.flush(); h.advance(250)
        assertEquals(1, retries)
        assertEquals(2, h.sockets.size)
    }

    @Test fun manualDisconnectInvalidatesPendingReconnectAndLateHello() {
        val h = Harness(); val socket = h.connect()
        var opens = 0; h.client.onOpen = { opens++ }
        socket.fail(); h.flush(); h.client.close()
        socket.hello(); h.advance(10000)
        assertEquals(1, h.sockets.size)
        assertEquals(0, opens)
    }

    @Test fun staleCallbacksCannotUpdateReplacementConnection() {
        val h = Harness(); val old = h.connect()
        var opens = 0; var retries = 0
        h.client.onOpen = { opens++ }; h.client.onReconnecting = { retries++ }
        h.client.connect("ws://localhost/other", "new-token")
        old.hello(); old.fail(); h.flush()
        assertEquals(0, opens); assertEquals(0, retries)
        h.sockets.last().hello(); h.flush()
        assertEquals(1, opens)
    }

    @Test fun queuedTerminalFrameIsIgnoredAfterExplicitClose() {
        val h = Harness(); val socket = h.connect()
        var frames = 0; h.client.onChannelData = { _, _ -> frames++ }
        socket.listener.onMessage(socket, ByteString.of(*Frame.encode(1, byteArrayOf(1))))
        h.client.close(); h.flush()
        assertEquals(0, frames)
    }
}
