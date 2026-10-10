package ai.eclosion.octoterm.android.connection

import java.net.URI
import java.net.URLDecoder
import java.nio.charset.StandardCharsets

data class ParsedEndpoint(
    val webSocketUrl: String,
    val token: String?,
    val host: String,
    val displayHost: String,
    val isLoopback: Boolean,
    val isWildcardBind: Boolean,
) {
    /** `GET /api/launchers` 用的源站。WebSocket 地址换成 http(s)，去掉路径。 */
    fun httpOrigin(): String {
        val uri = try {
            URI(webSocketUrl)
        } catch (_: Exception) {
            return ""
        }
        val scheme = if (uri.scheme == "wss") "https" else "http"
        val host = uri.host ?: return ""
        val hostForUrl = if (host.contains(':') && !host.startsWith("[")) "[$host]" else host
        val port = if (uri.port > 0) ":${uri.port}" else ""
        return "$scheme://$hostForUrl$port"
    }
}

class ConnectionUrlException(message: String) : IllegalArgumentException(message)

object ConnectionUrl {
    fun looksLikeUrl(raw: String): Boolean {
        val t = raw.trim()
        if (t.isEmpty()) return false
        val lower = t.lowercase()
        return lower.startsWith("http://") ||
            lower.startsWith("https://") ||
            lower.startsWith("ws://") ||
            lower.startsWith("wss://")
    }

    fun parse(raw: String, tokenOverride: String? = null): ParsedEndpoint {
        val trimmed = raw.trim()
        if (trimmed.isEmpty()) {
            throw ConnectionUrlException("empty")
        }
        val normalized = if (trimmed.contains("://")) trimmed else "http://$trimmed"
        val uri = try {
            URI(normalized)
        } catch (_: Exception) {
            throw ConnectionUrlException("malformed")
        }
        val scheme = uri.scheme?.lowercase()
            ?: throw ConnectionUrlException("malformed")
        val wsScheme = when (scheme) {
            "http", "ws" -> "ws"
            "https", "wss" -> "wss"
            else -> throw ConnectionUrlException("scheme")
        }
        val host = uri.host?.trim().orEmpty()
        if (host.isEmpty()) {
            throw ConnectionUrlException("host")
        }
        val portSuffix = if (uri.port > 0) ":${uri.port}" else ""
        val path = websocketPath(uri.rawPath)
        val token = tokenOverride?.trim()?.takeIf { it.isNotEmpty() }
            ?: tokenFrom(uri.rawQuery, uri.rawFragment)
        val hostForUrl = if (host.contains(':') && !host.startsWith("[")) "[$host]" else host
        val displayHost = hostForUrl + portSuffix
        return ParsedEndpoint(
            webSocketUrl = "$wsScheme://$hostForUrl$portSuffix$path",
            token = token,
            host = host,
            displayHost = displayHost,
            isLoopback = isLoopback(host),
            isWildcardBind = isWildcard(host),
        )
    }

    fun displayHost(rawUrl: String): String {
        return try {
            parse(rawUrl).displayHost
        } catch (_: ConnectionUrlException) {
            rawUrl.trim()
        }
    }

    private fun websocketPath(rawPath: String?): String {
        val path = rawPath?.takeIf { it.isNotEmpty() } ?: "/"
        return if (path == "/" || path.isEmpty()) {
            "/ws"
        } else if (path == "/ws" || path.endsWith("/ws")) {
            path
        } else {
            path.trimEnd('/') + "/ws"
        }
    }

    private fun tokenFrom(rawQuery: String?, rawFragment: String?): String? {
        return firstTokenParam(rawFragment) ?: firstTokenParam(rawQuery)
    }

    private fun firstTokenParam(encoded: String?): String? {
        if (encoded.isNullOrEmpty()) return null
        return encoded.split('&').firstNotNullOfOrNull { part ->
            val idx = part.indexOf('=')
            if (idx <= 0) return@firstNotNullOfOrNull null
            val key = part.substring(0, idx)
            if (key != "token") return@firstNotNullOfOrNull null
            decode(part.substring(idx + 1)).takeIf { it.isNotEmpty() }
        }
    }

    private fun decode(value: String): String {
        return URLDecoder.decode(value.replace('+', ' '), StandardCharsets.UTF_8.name())
    }

    private fun isLoopback(host: String): Boolean {
        val h = host.trim().lowercase().removePrefix("[").removeSuffix("]")
        return h == "localhost" || h == "127.0.0.1" || h == "::1"
    }

    private fun isWildcard(host: String): Boolean {
        val h = host.trim().lowercase().removePrefix("[").removeSuffix("]")
        return h == "0.0.0.0" || h == "::"
    }
}
