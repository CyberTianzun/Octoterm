package ai.eclosion.octoterm.android.wire

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.charset.StandardCharsets

class FrameAndControlTest {
    @Test
    fun frameRoundtrip() {
        val encoded = Frame.encode(7, "hello".toByteArray())
        val decoded = Frame.decode(encoded)
        assertEquals(7, decoded.channel)
        assertEquals(0, decoded.flags)
        assertEquals("hello", String(decoded.payload, StandardCharsets.UTF_8))
    }

    @Test
    fun helloEscapesTokenQuotes() {
        val bytes = ControlJson.encodeHello("a\"b")
        val decoded = Frame.decode(bytes)
        assertEquals(0, decoded.channel)
        val json = String(decoded.payload, StandardCharsets.UTF_8)
        assertTrue(json.contains("\"type\":\"hello\""))
        assertTrue(json.contains("\"token\":\"a\\\"b\""))
        assertTrue(json.contains("\"proto\":1"))
    }

    @Test
    fun parseHelloOkAndError() {
        val ok = ControlJson.parse("""{"type":"hello-ok","proto":1}""".toByteArray())
        assertEquals(ServerMsg.HelloOk(1), ok)
        assertEquals(ServerMsg.HelloOk(1, "windows"), ControlJson.parse(
            """{"type":"hello-ok","proto":1,"os":"windows"}""".toByteArray(),
        ))
        val err = ControlJson.parse("""{"type":"error","message":"bad hello"}""".toByteArray())
        assertEquals(ServerMsg.Error("bad hello", null), err)
    }

    @Test
    fun parseSessionsAndResized() {
        val sessions = ControlJson.parse(
            """{"type":"sessions","sessions":[{"id":2,"name":"zsh","cols":80,"rows":24,"created_at":1}]}""".toByteArray(),
        ) as ServerMsg.Sessions
        assertEquals(1, sessions.sessions.size)
        assertEquals(2L, sessions.sessions[0].id)
        val resized = ControlJson.parse("""{"type":"resized","channel":1,"cols":100,"rows":30}""".toByteArray())
        assertEquals(ServerMsg.Resized(1, 100, 30), resized)
    }

    @Test
    fun encodeAttachNullSeq() {
        val bytes = ControlJson.encode(ClientMsg.Attach(3, 1, null, 80, 24))
        val json = String(Frame.decode(bytes).payload, StandardCharsets.UTF_8)
        assertTrue(json.contains("\"type\":\"attach\""))
        assertTrue(json.contains("\"last_seq\":null"))
        assertNull((ControlJson.parse("""{"type":"error","message":"x"}""".toByteArray()) as ServerMsg.Error).channel)
    }

    @Test(expected = IllegalArgumentException::class)
    fun shortFrameRejected() {
        Frame.decode(byteArrayOf(1, 2, 3))
    }
}
