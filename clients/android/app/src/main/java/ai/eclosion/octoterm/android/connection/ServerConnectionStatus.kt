package ai.eclosion.octoterm.android.connection

enum class ServerConnectionStatus {
    Disconnected, Connecting, Connected, Reconnecting, Failed;
    val active: Boolean get() = this == Connecting || this == Connected || this == Reconnecting
}
