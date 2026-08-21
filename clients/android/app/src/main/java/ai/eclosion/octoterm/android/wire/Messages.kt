package ai.eclosion.octoterm.android.wire

data class SessionInfo(
    val id: Long,
    val name: String,
    val cols: Int,
    val rows: Int,
    val createdAt: Long,
)

enum class AttachMode { Replay, Resync }

enum class SessionEventKind { Created, Renamed, Closed }

sealed class ClientMsg {
    data class Hello(val token: String, val proto: Int = ControlJson.PROTO_VERSION) : ClientMsg()
    data object ListSessions : ClientMsg()
    data class NewSession(
        val name: String? = null,
        val command: List<String>? = null,
        val cwd: String? = null,
    ) : ClientMsg()

    data class KillSession(val id: Long) : ClientMsg()
    data class RenameSession(val id: Long, val name: String) : ClientMsg()
    data class Attach(
        val id: Long,
        val channel: Int,
        val lastSeq: Long?,
        val cols: Int,
        val rows: Int,
    ) : ClientMsg()

    data class Detach(val channel: Int) : ClientMsg()
    data class Resize(val channel: Int, val cols: Int, val rows: Int) : ClientMsg()
}

sealed class ServerMsg {
    data class HelloOk(val proto: Int) : ServerMsg()
    data class Error(val message: String, val channel: Int?) : ServerMsg()
    data class Sessions(val sessions: List<SessionInfo>) : ServerMsg()
    data class SessionEvent(val event: SessionEventKind, val session: SessionInfo) : ServerMsg()
    data class Attached(val channel: Int, val seq: Long, val mode: AttachMode) : ServerMsg()
    data class Resized(val channel: Int, val cols: Int, val rows: Int) : ServerMsg()
    data class ResyncBegin(val channel: Int) : ServerMsg()
    data class ResyncEnd(val channel: Int, val seq: Long) : ServerMsg()
    data class SessionExited(val channel: Int, val id: Long) : ServerMsg()
    data class Unknown(val type: String) : ServerMsg()
}
