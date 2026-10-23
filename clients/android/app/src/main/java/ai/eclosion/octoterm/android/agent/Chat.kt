package ai.eclosion.octoterm.android.agent

import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject

/**
 * 聊天视图的数据层。
 *
 * 消息来自 agent 自己写的对话记录(服务端读、归一化),走 HTTP 拉取 —— 一段对话可以是
 * 几 MB,协议 R4 不许在控制通道走大块数据。**增量由 `agent-event` 触发**:有事件才可能
 * 有新消息,所以不做轮询。和 web 端 `chat.ts` 一一对应。
 */
enum class ChatRole { User, Assistant, System }

sealed class ChatBlock {
    data class Text(val text: String) : ChatBlock()
    data class Thinking(val text: String) : ChatBlock()
    /** `input` 是服务端给的「给人看的一行」,不是原始 JSON */
    data class ToolUse(val name: String, val input: String) : ChatBlock()
    data class ToolResult(val ok: Boolean, val text: String) : ChatBlock()
}

data class ChatMessage(
    val id: String,
    val role: ChatRole,
    val ts: Long?,
    val blocks: List<ChatBlock>,
)

/** 读不到对话记录时的原因。服务端给的是机器可读的键,文案在 UI 层按语言渲染。 */
enum class ChatFallback(val wire: String) {
    Disabled("disabled"),
    NoTranscriptPath("no-transcript-path"),
    UnsupportedAgent("unsupported-agent"),
    Unreadable("unreadable"),
    ParseFailed("parse-failed"),
    ;

    companion object {
        /** 认不出的原因也要说人话 —— 归到「读不到」,而不是把机器键怼给用户 */
        fun parse(s: String): ChatFallback = entries.firstOrNull { it.wire == s } ?: Unreadable
    }
}

sealed class ChatWindow {
    /** `cursor` 是不透明的:原样带回,**不解析**(协议 A9) */
    data class Transcript(
        val messages: List<ChatMessage>,
        val cursor: String,
        val reset: Boolean,
        val more: Boolean,
        val olderCursor: String? = null,
        val historySupported: Boolean = false,
    ) : ChatWindow()

    data class Terminal(val reason: ChatFallback) : ChatWindow()
}

object Chat {
    fun parseWindow(body: String): ChatWindow {
        val root = try {
            JSONObject(body)
        } catch (_: Exception) {
            return ChatWindow.Terminal(ChatFallback.Unreadable)
        }
        if (root.optString("source") != "transcript") {
            return ChatWindow.Terminal(ChatFallback.parse(root.optString("reason")))
        }
        val array = root.optJSONArray("messages")
        val messages = buildList {
            if (array != null) {
                for (i in 0 until array.length()) {
                    array.optJSONObject(i)?.let(::parseMessage)?.let(::add)
                }
            }
        }
        return ChatWindow.Transcript(
            messages = messages,
            cursor = root.optString("cursor"),
            reset = root.optBoolean("reset"),
            more = root.optBoolean("more"),
            olderCursor = root.optString("older_cursor").takeUnless { it.isEmpty() || it == "null" },
            historySupported = root.has("older_cursor"),
        )
    }

    private fun parseMessage(obj: JSONObject): ChatMessage? {
        val id = obj.optString("id").ifEmpty { return null }
        val role = when (obj.optString("role")) {
            "user" -> ChatRole.User
            "assistant" -> ChatRole.Assistant
            else -> ChatRole.System
        }
        val array = obj.optJSONArray("blocks")
        val blocks = buildList {
            if (array != null) {
                for (i in 0 until array.length()) {
                    val b = array.optJSONObject(i) ?: continue
                    // 认不出的块跳过:新版服务端加了块类型,旧客户端照样能看其余部分
                    when (b.optString("kind")) {
                        "text" -> add(ChatBlock.Text(b.optString("text")))
                        "thinking" -> add(ChatBlock.Thinking(b.optString("text")))
                        "tool-use" -> add(ChatBlock.ToolUse(b.optString("name"), b.optString("input")))
                        "tool-result" -> add(ChatBlock.ToolResult(b.optBoolean("ok", true), b.optString("text")))
                    }
                }
            }
        }
        val ts = if (obj.has("ts") && !obj.isNull("ts")) obj.optLong("ts") else null
        return ChatMessage(id, role, ts, blocks)
    }

    /**
     * 把一次拉取合并进已有的消息。
     *
     * `reset` 是**整段替换**:服务端认定旧游标失效(记录被 compact 了、或换了会话),旧内容和
     * 新内容不是同一条时间线。追加时按 id 去重:窗口边界上的那条可能被重复送来。
     *
     * 同一批里也去重:拿不到 agent 自己的 id 时服务端按内容合成,两条一模一样的消息会撞 id,
     * 而列表按 id 作 key,重复的 key 会让界面直接崩掉。
     */
    fun merge(prev: List<ChatMessage>, win: ChatWindow, prepend: Boolean = false): List<ChatMessage> {
        if (win !is ChatWindow.Transcript) return prev
        val base = if (win.reset) emptyList() else prev
        val seen = base.mapTo(HashSet()) { it.id }
        val incoming = win.messages.filter { seen.add(it.id) }
        return if (prepend) incoming + base else base + incoming
    }

    /** 老版本服务端没有这个路由:降级成「读不到」,不是报错(协议 T12)。 */
    fun fetch(
        http: OkHttpClient,
        origin: String,
        token: String,
        agentId: String,
        agentSessionId: String,
        after: String?,
        before: String? = null,
    ): ChatWindow {
        val unreadable = ChatWindow.Terminal(ChatFallback.Unreadable)
        return try {
            val url = (origin.trimEnd('/') + "/api/agents/messages").toHttpUrlOrNull()
                ?.newBuilder()
                ?.addQueryParameter("agent_id", agentId)
                ?.addQueryParameter("agent_session_id", agentSessionId)
                ?.apply { if (after != null) addQueryParameter("after", after) }
                ?.apply { if (before != null) addQueryParameter("before", before) }
                ?.build()
                ?: return unreadable
            val request = Request.Builder().url(url).header("Authorization", "Bearer $token").build()
            http.newCall(request).execute().use { response ->
                if (!response.isSuccessful) return unreadable
                parseWindow(response.body?.string().orEmpty())
            }
        } catch (_: Exception) {
            unreadable
        }
    }
}
