package ai.eclosion.octoterm.android.connection

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ConnectionUrlTest {
    @Test
    fun parsesDaemonStartupLink() {
        val parsed = ConnectionUrl.parse("http://192.168.1.10:7683/#token=s3cret")
        assertEquals("ws://192.168.1.10:7683/ws", parsed.webSocketUrl)
        assertEquals("s3cret", parsed.token)
        assertEquals("192.168.1.10:7683", parsed.displayHost)
        assertFalse(parsed.isLoopback)
    }

    @Test
    fun acceptsBareHostPort() {
        val parsed = ConnectionUrl.parse("10.0.0.2:9000", "abc")
        assertEquals("ws://10.0.0.2:9000/ws", parsed.webSocketUrl)
        assertEquals("abc", parsed.token)
    }

    @Test
    fun httpsBecomesWss() {
        val parsed = ConnectionUrl.parse("https://term.example/#token=x")
        assertEquals("wss://term.example/ws", parsed.webSocketUrl)
        assertEquals("x", parsed.token)
    }

    @Test
    fun keepsExplicitWsPath() {
        val parsed = ConnectionUrl.parse("ws://host.internal:7683/ws#token=t")
        assertEquals("ws://host.internal:7683/ws", parsed.webSocketUrl)
    }

    @Test
    fun overrideWinsOverFragment() {
        val parsed = ConnectionUrl.parse("http://h:1/#token=old", "new")
        assertEquals("new", parsed.token)
    }

    @Test
    fun readsQueryTokenIfSomeonePastesIt() {
        val parsed = ConnectionUrl.parse("http://h:1/?token=from-query")
        assertEquals("from-query", parsed.token)
    }

    @Test
    fun flagsLoopbackAndWildcard() {
        assertTrue(ConnectionUrl.parse("http://127.0.0.1:7683/#token=a").isLoopback)
        assertTrue(ConnectionUrl.parse("http://localhost:7683/#token=a").isLoopback)
        assertTrue(ConnectionUrl.parse("http://0.0.0.0:7683/#token=a").isWildcardBind)
        assertNull(ConnectionUrl.parse("http://192.168.0.4:7683/").token)
    }

    @Test
    fun looksLikeUrl() {
        assertTrue(ConnectionUrl.looksLikeUrl("  https://x/#token=1 "))
        assertFalse(ConnectionUrl.looksLikeUrl("not a url"))
        assertFalse(ConnectionUrl.looksLikeUrl(""))
    }

    @Test(expected = ConnectionUrlException::class)
    fun rejectsEmpty() {
        ConnectionUrl.parse("   ")
    }
}
