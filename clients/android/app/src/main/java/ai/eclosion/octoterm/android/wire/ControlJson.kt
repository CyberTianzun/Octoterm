package ai.eclosion.octoterm.android.wire

import org.json.JSONArray
import org.json.JSONObject
import java.nio.charset.StandardCharsets

object ControlJson {
    const val PROTO_VERSION: Int = 1

    fun encode(msg: ClientMsg): ByteArray {
        val obj = JSONObject()
        when (msg) {
            is ClientMsg.Hello -> {
                obj.put("type", "hello").put("token", msg.token).put("proto", msg.proto)
            }
            ClientMsg.ListSessions -> obj.put("type", "list-sessions")
            is ClientMsg.NewSession -> {
                obj.put("type", "new-session")
                obj.put("name", msg.name ?: JSONObject.NULL)
                if (msg.command == null) {
                    obj.put("command", JSONObject.NULL)
                } else {
                    obj.put("command", JSONArray(msg.command))
                }
                obj.put("cwd", msg.cwd ?: JSONObject.NULL)
            }
            is ClientMsg.KillSession -> obj.put("type", "kill-session").put("id", msg.id)
            is ClientMsg.RenameSession ->
                obj.put("type", "rename-session").put("id", msg.id).put("name", msg.name)
            is ClientMsg.Attach -> {
                obj.put("type", "attach")
                    .put("id", msg.id)
                    .put("channel", msg.channel)
                    .put("last_seq", msg.lastSeq ?: JSONObject.NULL)
                    .put("cols", msg.cols)
                    .put("rows", msg.rows)
            }
            is ClientMsg.Detach -> obj.put("type", "detach").put("channel", msg.channel)
            is ClientMsg.Resize ->
                obj.put("type", "resize").put("channel", msg.channel).put("cols", msg.cols).put("rows", msg.rows)
        }
        return Frame.encode(Frame.CONTROL_CHANNEL, obj.toString().toByteArray(StandardCharsets.UTF_8))
    }

    fun encodeHello(token: String): ByteArray = encode(ClientMsg.Hello(token))

    fun parse(payload: ByteArray): ServerMsg {
        val obj = JSONObject(String(payload, StandardCharsets.UTF_8))
        return when (val type = obj.optString("type")) {
            "hello-ok" -> ServerMsg.HelloOk(obj.optInt("proto", PROTO_VERSION))
            "error" -> ServerMsg.Error(
                message = obj.optString("message"),
                channel = if (obj.has("channel") && !obj.isNull("channel")) obj.getInt("channel") else null,
            )
            "sessions" -> ServerMsg.Sessions(parseSessions(obj.optJSONArray("sessions")))
            "session-event" -> ServerMsg.SessionEvent(
                event = when (obj.optString("event")) {
                    "renamed" -> SessionEventKind.Renamed
                    "closed" -> SessionEventKind.Closed
                    else -> SessionEventKind.Created
                },
                session = parseSession(obj.getJSONObject("session")),
            )
            "attached" -> ServerMsg.Attached(
                channel = obj.getInt("channel"),
                seq = obj.optLong("seq"),
                mode = if (obj.optString("mode") == "replay") AttachMode.Replay else AttachMode.Resync,
            )
            "resized" -> ServerMsg.Resized(
                channel = obj.getInt("channel"),
                cols = obj.getInt("cols"),
                rows = obj.getInt("rows"),
            )
            "resync-begin" -> ServerMsg.ResyncBegin(obj.getInt("channel"))
            "resync-end" -> ServerMsg.ResyncEnd(obj.getInt("channel"), obj.optLong("seq"))
            "session-exited" -> ServerMsg.SessionExited(obj.getInt("channel"), obj.getLong("id"))
            else -> ServerMsg.Unknown(type)
        }
    }

    private fun parseSessions(array: JSONArray?): List<SessionInfo> {
        if (array == null) return emptyList()
        return buildList {
            for (i in 0 until array.length()) {
                add(parseSession(array.getJSONObject(i)))
            }
        }
    }

    private fun parseSession(obj: JSONObject): SessionInfo {
        return SessionInfo(
            id = obj.getLong("id"),
            name = obj.optString("name"),
            cols = obj.optInt("cols"),
            rows = obj.optInt("rows"),
            createdAt = obj.optLong("created_at"),
        )
    }
}
