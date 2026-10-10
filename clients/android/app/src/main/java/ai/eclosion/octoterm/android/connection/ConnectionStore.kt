package ai.eclosion.octoterm.android.connection

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

class ConnectionStore(context: Context) {
    private val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun load(): List<ServerConnection> {
        val raw = prefs.getString(KEY, null) ?: return emptyList()
        return try {
            val array = JSONArray(raw)
            buildList {
                for (i in 0 until array.length()) {
                    val obj = array.optJSONObject(i) ?: continue
                    val id = obj.optString("id")
                    val url = obj.optString("url")
                    if (id.isEmpty() || url.isEmpty()) continue
                    add(
                        ServerConnection(
                            id = id,
                            name = obj.optString("name"),
                            url = url,
                            token = obj.optString("token"),
                            createdAt = obj.optLong("createdAt", 0L),
                            lastUsedAt = obj.optLong("lastUsedAt", 0L).takeIf { it > 0L },
                        ),
                    )
                }
            }
        } catch (_: Exception) {
            emptyList()
        }
    }

    fun save(items: List<ServerConnection>) {
        val array = JSONArray()
        for (item in items) {
            array.put(
                JSONObject()
                    .put("id", item.id)
                    .put("name", item.name)
                    .put("url", item.url)
                    .put("token", item.token)
                    .put("createdAt", item.createdAt)
                    .put("lastUsedAt", item.lastUsedAt ?: 0L),
            )
        }
        prefs.edit().putString(KEY, array.toString()).apply()
    }

    private companion object {
        const val PREFS = "octoterm.connections"
        const val KEY = "items"
    }
}
