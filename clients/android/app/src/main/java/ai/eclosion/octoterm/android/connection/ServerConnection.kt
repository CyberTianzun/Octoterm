package ai.eclosion.octoterm.android.connection

data class ServerConnection(
    val id: String,
    val name: String,
    val url: String,
    val token: String,
    val createdAt: Long,
    val lastUsedAt: Long? = null,
) {
    fun title(): String {
        val trimmed = name.trim()
        return trimmed.ifEmpty { ConnectionUrl.displayHost(url) }
    }

    fun subtitle(): String = ConnectionUrl.displayHost(url)
}
