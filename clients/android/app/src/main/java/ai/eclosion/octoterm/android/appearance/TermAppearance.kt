package ai.eclosion.octoterm.android.appearance

import ai.eclosion.octoterm.android.term.Color
import org.json.JSONObject

enum class TermFontKind { Mono, Serif, Sans }

/**
 * 纯客户端外观。和 web 的 OctoConfig 同一份 JSON：颜色、字号、回滚行数。
 * 导入的文本不可信，坏字段退回 [base]，整段不是 JSON 时返回 null。
 */
data class TermAppearance(
    val name: String,
    val foreground: Int,
    val background: Int,
    val cursor: Int,
    val selection: Int,
    val ansi: List<Int>,
    val fontSizeSp: Float,
    val font: TermFontKind,
    val scrollback: Int,
) {
    fun colorFor(index: Int, stored: Int, foreground: Boolean): Int {
        return when (index) {
            Color.INDEX_DEFAULT -> if (foreground) this.foreground else background
            Color.INDEX_RGB -> stored and 0xFFFFFF
            in 0..15 -> ansi.getOrElse(index) { stored and 0xFFFFFF }
            in 16..255 -> Color.indexed(index)
            else -> stored and 0xFFFFFF
        }
    }

    fun withFontSize(size: Float): TermAppearance = copy(fontSizeSp = size.coerceIn(8f, 32f))

    fun withScrollback(lines: Int): TermAppearance = copy(scrollback = lines.coerceIn(0, 200_000))

    fun withFont(kind: TermFontKind): TermAppearance = copy(font = kind)

    fun withTheme(theme: TermAppearance): TermAppearance = theme.copy(
        fontSizeSp = fontSizeSp,
        font = font,
        scrollback = scrollback,
    )
}

object AppearanceCodec {
    const val VERSION = 1

    fun default(prefersDark: Boolean = true): TermAppearance = if (prefersDark) dark2026() else light2026()

    fun builtin(name: String): TermAppearance? = when (name) {
        "Tokyo Night" -> tokyoNight()
        "2026 Dark" -> dark2026()
        "2026 Light" -> light2026()
        else -> null
    }

    fun builtins(): List<TermAppearance> = listOf(light2026(), dark2026(), tokyoNight())

    fun exportJson(appearance: TermAppearance): String {
        val colors = JSONObject()
        colors.put("foreground", hex(appearance.foreground))
        colors.put("background", hex(appearance.background))
        colors.put("cursor", hex(appearance.cursor))
        colors.put("selectionBackground", hexCss(appearance.selection))
        ANSI_KEYS.forEachIndexed { i, key ->
            colors.put(key, hex(appearance.ansi.getOrElse(i) { 0 }))
        }
        val theme = JSONObject()
            .put("name", appearance.name)
            .put("colors", colors)
        val font = JSONObject()
            .put("family", fontFamily(appearance.font))
            .put("size", appearance.fontSizeSp.toDouble())
        val terminal = JSONObject().put("scrollback", appearance.scrollback)
        return JSONObject()
            .put("version", VERSION)
            .put("theme", theme)
            .put("font", font)
            .put("terminal", terminal)
            .toString(2)
    }

    /** 不是 JSON 对象时返回 null。字段缺失或非法则沿用 [base]。 */
    fun importJson(text: String, base: TermAppearance = default()): TermAppearance? {
        val root = try {
            JSONObject(text)
        } catch (_: Exception) {
            return null
        }
        val theme = root.optJSONObject("theme")
        val name = theme?.optString("name")?.trim()?.take(80).orEmpty().ifEmpty { base.name }
        val named = builtin(name)
        val colors = theme?.optJSONObject("colors")
        var next = when {
            colors != null -> fromColors(name, colors, base)
            named != null -> named.copy(fontSizeSp = base.fontSizeSp, font = base.font, scrollback = base.scrollback)
            else -> base.copy(name = name)
        }
        val font = root.optJSONObject("font")
        if (font != null) {
            if (font.has("size")) next = next.withFontSize(font.optDouble("size", next.fontSizeSp.toDouble()).toFloat())
            if (font.has("family")) next = next.withFont(fontKind(font.optString("family")))
        }
        val terminal = root.optJSONObject("terminal")
        if (terminal != null && terminal.has("scrollback")) {
            next = next.withScrollback(terminal.optInt("scrollback", next.scrollback))
        }
        return next
    }

    private fun fromColors(name: String, colors: JSONObject, base: TermAppearance): TermAppearance {
        val fg = parseRgb(colors.optString("foreground")) ?: base.foreground
        val bg = parseRgb(colors.optString("background")) ?: base.background
        val cursor = parseRgb(colors.optString("cursor")) ?: base.cursor
        val selection = parseCss(colors.optString("selectionBackground")) ?: base.selection
        val ansi = ANSI_KEYS.mapIndexed { i, key ->
            if (!colors.has(key)) base.ansi.getOrElse(i) { 0 }
            else parseRgb(colors.optString(key)) ?: base.ansi.getOrElse(i) { 0 }
        }
        return base.copy(
            name = name,
            foreground = fg,
            background = bg,
            cursor = cursor,
            selection = selection,
            ansi = ansi,
        )
    }

    private fun tokyoNight(): TermAppearance = TermAppearance(
        name = "Tokyo Night",
        foreground = Color.DEFAULT_FG and 0xFFFFFF,
        background = Color.DEFAULT_BG and 0xFFFFFF,
        cursor = 0x7AA2F7,
        selection = 0x66334467,
        ansi = listOf(
            0x151520, 0xF7768E, 0x9ECE6A, 0xE0AF68,
            0x7AA2F7, 0xBB9AF7, 0x7DCFFF, 0xC0CAF5,
            0x565F89, 0xF7768E, 0x9ECE6A, 0xE0AF68,
            0x7AA2F7, 0xBB9AF7, 0x7DCFFF, 0xC0CAF5,
        ),
        fontSizeSp = 14f,
        font = TermFontKind.Mono,
        scrollback = 20_000,
    )

    private fun dark2026(): TermAppearance = TermAppearance(
        name = "2026 Dark",
        foreground = 0xCCCCCC,
        background = 0x191A1B,
        cursor = 0xBFBFBF,
        selection = 0x333994BC,
        ansi = listOf(
            0x000000, 0xCD3131, 0x0DBC79, 0xE5E510,
            0x2472C8, 0xBC3FBC, 0x11A8CD, 0xE5E5E5,
            0x666666, 0xF14C4C, 0x23D18B, 0xF5F543,
            0x3B8EEA, 0xD670D6, 0x29B8DB, 0xE5E5E5,
        ),
        fontSizeSp = 14f,
        font = TermFontKind.Mono,
        scrollback = 20_000,
    )

    private fun light2026(): TermAppearance = TermAppearance(
        name = "2026 Light",
        foreground = 0x3B3B3B,
        background = 0xFAFAFD,
        cursor = 0x202020,
        selection = 0x260069CC,
        ansi = listOf(
            0x000000, 0xCD3131, 0x107C10, 0x949800,
            0x0451A5, 0xBC05BC, 0x0598BC, 0x555555,
            0x666666, 0xCD3131, 0x14CE14, 0xB5BA00,
            0x0451A5, 0xBC05BC, 0x0598BC, 0xA5A5A5,
        ),
        fontSizeSp = 14f,
        font = TermFontKind.Mono,
        scrollback = 20_000,
    )

    private fun fontFamily(kind: TermFontKind): String = when (kind) {
        TermFontKind.Mono -> "monospace"
        TermFontKind.Serif -> "serif"
        TermFontKind.Sans -> "sans-serif"
    }

    private fun fontKind(family: String): TermFontKind {
        val lower = family.lowercase()
        return when {
            "mono" in lower -> TermFontKind.Mono
            "sans" in lower -> TermFontKind.Sans
            "serif" in lower -> TermFontKind.Serif
            else -> TermFontKind.Mono
        }
    }

    private fun hex(rgb: Int): String = "#%06X".format(rgb and 0xFFFFFF)

    private fun hexCss(argb: Int): String {
        val rgb = argb and 0xFFFFFF
        val alpha = (argb ushr 24) and 0xFF
        return if (alpha == 0xFF || alpha == 0) hex(rgb) else "#%06X%02X".format(rgb, alpha)
    }

    /** #RRGGBB，忽略 alpha。 */
    fun parseRgb(raw: String?): Int? {
        val argb = parseCss(raw) ?: return null
        return argb and 0xFFFFFF
    }

    /** CSS 顺序：#RGB、#RRGGBB、#RRGGBBAA。返回 Android 的 AARRGGBB。 */
    fun parseCss(raw: String?): Int? {
        if (raw.isNullOrBlank()) return null
        val body = raw.trim().removePrefix("#")
        if (body.any { it !in '0'..'9' && it !in 'a'..'f' && it !in 'A'..'F' }) return null
        val expanded = when (body.length) {
            3, 4 -> body.map { "$it$it" }.joinToString("")
            6, 8 -> body
            else -> return null
        }
        return when (expanded.length) {
            6 -> (expanded.toLongOrNull(16) ?: return null).toInt() or 0xFF000000.toInt()
            8 -> {
                val rgb = expanded.substring(0, 6).toLongOrNull(16) ?: return null
                val alpha = expanded.substring(6, 8).toIntOrNull(16) ?: return null
                (alpha shl 24) or rgb.toInt()
            }
            else -> null
        }
    }

    private val ANSI_KEYS = listOf(
        "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
        "brightBlack", "brightRed", "brightGreen", "brightYellow",
        "brightBlue", "brightMagenta", "brightCyan", "brightWhite",
    )
}
