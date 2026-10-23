package ai.eclosion.octoterm.android.agent

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatTest {
    private fun msg(id: String, text: String = id) =
        ChatMessage(id, ChatRole.Assistant, null, listOf(ChatBlock.Text(text)))

    @Test
    fun olderPageMetadataIsOptionalForOlderServers() {
        val old = Chat.parseWindow("""{"source":"transcript","messages":[],"cursor":"c","reset":true,"more":false}""") as ChatWindow.Transcript
        assertEquals(false, old.historySupported)
        assertEquals(null, old.olderCursor)
        val page = Chat.parseWindow("""{"source":"transcript","messages":[],"older_cursor":"opaque"}""") as ChatWindow.Transcript
        assertEquals(true, page.historySupported)
        assertEquals("opaque", page.olderCursor)
        val beginning = Chat.parseWindow("""{"source":"transcript","messages":[],"older_cursor":null}""") as ChatWindow.Transcript
        assertEquals(true, beginning.historySupported)
        assertEquals(null, beginning.olderCursor)
    }

    @Test
    fun earlierMessagesPrependAndKeepTheExistingLiveVersionAtOverlaps() {
        val win = ChatWindow.Transcript(listOf(msg("a"), msg("b"), msg("c"), msg("b")), "forward-unused", false, false)
        val result = Chat.merge(listOf(msg("c", "latest c"), msg("d")), win, prepend = true)
        assertEquals(listOf("a", "b", "c", "d"), result.map { it.id })
        assertEquals(listOf(ChatBlock.Text("latest c")), result[2].blocks)
        assertEquals(listOf("a", "b", "c"), Chat.merge(listOf(msg("old")), win.copy(reset = true), prepend = true).map { it.id })
    }

    @Test
    fun parsesAllBlockKindsAndSkipsUnknownOnes() {
        val win = Chat.parseWindow(
            """{"source":"transcript","cursor":"c1","reset":false,"more":true,"messages":[
                {"id":"m1","role":"user","ts":1791957275,"blocks":[{"kind":"text","text":"hi"}]},
                {"id":"m2","role":"assistant","ts":null,"blocks":[
                  {"kind":"thinking","text":"hmm"},
                  {"kind":"tool-use","name":"Bash","input":"ls -la"},
                  {"kind":"tool-result","ok":false,"text":"boom"},
                  {"kind":"image","url":"x"}
                ]},
                {"role":"user","blocks":[]}
            ]}""",
        ) as ChatWindow.Transcript
        assertEquals("c1", win.cursor)
        assertTrue(win.more)
        assertEquals(listOf("m1", "m2"), win.messages.map { it.id })
        assertEquals(ChatRole.User, win.messages[0].role)
        assertEquals(1791957275L, win.messages[0].ts)
        assertEquals(
            listOf(
                ChatBlock.Thinking("hmm"),
                ChatBlock.ToolUse("Bash", "ls -la"),
                ChatBlock.ToolResult(false, "boom"),
            ),
            win.messages[1].blocks,
        )
    }

    @Test
    fun fallbackReasonsAreTypedAndUnknownOnesReadAsUnreadable() {
        assertEquals(
            ChatWindow.Terminal(ChatFallback.UnsupportedAgent),
            Chat.parseWindow("""{"source":"terminal","reason":"unsupported-agent"}"""),
        )
        assertEquals(
            ChatWindow.Terminal(ChatFallback.Unreadable),
            Chat.parseWindow("""{"source":"terminal","reason":"something-new"}"""),
        )
        assertEquals(ChatWindow.Terminal(ChatFallback.Unreadable), Chat.parseWindow("<html>"))
    }

    @Test
    fun appendDedupesOnTheWindowBoundary() {
        val prev = listOf(msg("a"), msg("b"))
        val win = ChatWindow.Transcript(listOf(msg("b"), msg("c")), "x", reset = false, more = false)
        assertEquals(listOf("a", "b", "c"), Chat.merge(prev, win).map { it.id })
    }

    /** reset = 旧时间线作废,整段替换而不是接在后面 */
    @Test
    fun resetReplacesEverything() {
        val prev = listOf(msg("a"), msg("b"))
        val win = ChatWindow.Transcript(listOf(msg("z")), "x", reset = true, more = false)
        assertEquals(listOf("z"), Chat.merge(prev, win).map { it.id })
    }

    /** 列表按 id 作 key,重复 id 会崩 —— 同一批里、reset 时都要去重 */
    @Test
    fun duplicateIdsWithinOneWindowAreDropped() {
        val dup = ChatWindow.Transcript(listOf(msg("a"), msg("a"), msg("b")), "x", reset = true, more = false)
        assertEquals(listOf("a", "b"), Chat.merge(listOf(msg("z")), dup).map { it.id })
        val append = dup.copy(reset = false)
        assertEquals(listOf("z", "a", "b"), Chat.merge(listOf(msg("z")), append).map { it.id })
    }

    @Test
    fun fallbackWindowLeavesMessagesAlone() {
        val prev = listOf(msg("a"))
        assertEquals(prev, Chat.merge(prev, ChatWindow.Terminal(ChatFallback.Disabled)))
    }
}
