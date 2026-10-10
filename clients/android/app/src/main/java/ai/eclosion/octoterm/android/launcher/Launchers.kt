package ai.eclosion.octoterm.android.launcher

import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject

/**
 * 新建会话菜单里的一条启动项。清单来自 `GET /api/launchers`，
 * 客户端只显示，并把选中的 argv 原样发回去。
 */
data class Launcher(
    val id: String,
    val provider: String,
    val name: String,
    val detail: String,
    val command: List<String>,
    val cwd: String?,
)

object Launchers {
    fun fallback(): Launcher = Launcher(
        id = "fallback:default",
        provider = "builtin",
        name = "",
        detail = "",
        command = emptyList(),
        cwd = null,
    )

    /** 结构不完整的条目丢掉。空 argv 也丢掉 —— 那是拉取失败时兜底项的约定，不来自服务端。 */
    fun sanitize(body: String): List<Launcher> {
        val root = try {
            JSONObject(body)
        } catch (_: Exception) {
            return emptyList()
        }
        val array = root.optJSONArray("launchers") ?: return emptyList()
        val out = ArrayList<Launcher>(array.length())
        for (i in 0 until array.length()) {
            val item = array.optJSONObject(i) ?: continue
            val id = item.optString("id")
            val name = item.optString("name")
            if (id.isEmpty() || name.isEmpty()) continue
            val commandJson = item.optJSONArray("command") ?: continue
            val command = ArrayList<String>(commandJson.length())
            var bad = false
            for (c in 0 until commandJson.length()) {
                val value = commandJson.opt(c)
                if (value !is String) {
                    bad = true
                    break
                }
                command.add(value)
            }
            if (bad || command.isEmpty()) continue
            val cwd = item.optString("cwd").takeIf { it.isNotEmpty() }
            val detail = item.optString("detail").ifEmpty { command.joinToString(" ") }
            out.add(
                Launcher(
                    id = id,
                    provider = item.optString("provider").ifEmpty { "unknown" },
                    name = name,
                    detail = detail,
                    command = command,
                    cwd = cwd,
                ),
            )
        }
        return out
    }

    fun fetch(http: OkHttpClient, origin: String, token: String): List<Launcher> {
        val fallback = listOf(fallback())
        return try {
            val request = Request.Builder()
                .url(origin.trimEnd('/') + "/api/launchers")
                .header("Authorization", "Bearer $token")
                .build()
            http.newCall(request).execute().use { response ->
                if (!response.isSuccessful) return fallback
                val list = sanitize(response.body?.string().orEmpty())
                list.ifEmpty { fallback }
            }
        } catch (_: Exception) {
            fallback
        }
    }
}
