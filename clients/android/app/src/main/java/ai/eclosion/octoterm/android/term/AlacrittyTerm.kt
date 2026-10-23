package ai.eclosion.octoterm.android.term

import java.nio.ByteBuffer
import java.nio.ByteOrder
import uniffi.octoterm_cloud_ffi.TermEngine
import uniffi.octoterm_cloud_ffi.TermState

/**
 * 基于 Rust `alacritty_terminal` 的客户端终端会话封装（通过 UniFFI `TermEngine` 驱动）。
 */
class AlacrittyTerm(
    cols: Int = 80,
    rows: Int = 24,
    maxScrollback: Int = 20_000,
) : AutoCloseable {
    var cols: Int = cols.coerceAtLeast(1)
        private set
    var rows: Int = rows.coerceAtLeast(1)
        private set

    var cursorX: Int = 0
        private set
    var cursorY: Int = 0
        private set
    var cursorVisible: Boolean = true
        private set
    var applicationCursor: Boolean = false
        private set
    var bracketedPaste: Boolean = false
        private set
    /** XTMODKEYS(`CSI > 4 ; Pv m`)。TerminalKeyboard 据此编码 Shift+Enter 等组合。 */
    var modifyOtherKeys: Int = 0
        private set

    /** 0 表示贴着最新一行。正数是向上翻了多少行。alt-screen 没有回滚。 */
    var scrollOffset: Int = 0
        private set

    var mouseTracking: Boolean = false
        private set

    @Volatile
    var generation: Long = 0L
        private set

    private val lock = Any()
    private val engine = TermEngine(
        this.cols.toUInt(),
        this.rows.toUInt(),
        maxScrollback.coerceIn(0, 200_000).toUInt(),
    )

    init {
        applyState(engine.state())
    }

    private fun applyState(st: TermState) {
        cols = st.cols.toInt()
        rows = st.rows.toInt()
        cursorX = st.cursorX.toInt()
        cursorY = st.cursorY.toInt()
        cursorVisible = st.cursorVisible
        applicationCursor = st.applicationCursor
        bracketedPaste = st.bracketedPaste
        modifyOtherKeys = st.modifyOtherKeys.toInt()
        scrollOffset = st.scrollOffset.toInt()
        mouseTracking = st.mouseTracking
    }

    fun snapshot(): Array<Array<Cell>> = synchronized(lock) {
        val r = rows
        val c = cols
        val packed = engine.snapshotPacked()
        val buf = ByteBuffer.wrap(packed).order(ByteOrder.LITTLE_ENDIAN)
        Array(r) {
            Array(c) {
                if (buf.remaining() >= PACKED_CELL_BYTES) unpackCell(buf) else Cell.blank()
            }
        }
    }

    fun cell(x: Int, y: Int): Cell = synchronized(lock) {
        if (x !in 0 until cols || y !in 0 until rows) return Cell.blank()
        val packed = engine.cellPacked(x.toUInt(), y.toUInt())
        val buf = ByteBuffer.wrap(packed).order(ByteOrder.LITTLE_ENDIAN)
        if (buf.remaining() >= PACKED_CELL_BYTES) unpackCell(buf) else Cell.blank()
    }

    fun reset() = synchronized(lock) {
        applyState(engine.reset())
        generation++
    }

    fun resize(newCols: Int, newRows: Int) {
        val c = newCols.coerceAtLeast(1)
        val r = newRows.coerceAtLeast(1)
        synchronized(lock) {
            if (c == cols && r == rows) return
            applyState(engine.resize(c.toUInt(), r.toUInt()))
            generation++
        }
    }

    fun write(bytes: ByteArray) = synchronized(lock) {
        applyState(engine.write(bytes))
        generation++
    }

    fun scrollBy(delta: Int) {
        if (delta == 0) return
        synchronized(lock) {
            val prev = scrollOffset
            applyState(engine.scrollBy(delta))
            if (scrollOffset != prev) generation++
        }
    }

    fun scrollToBottom() = synchronized(lock) {
        if (scrollOffset != 0) {
            applyState(engine.scrollToBottom())
            generation++
        }
    }

    fun setMaxScrollback(limit: Int) = synchronized(lock) {
        applyState(engine.setMaxScrollback(limit.coerceIn(0, 200_000).toUInt()))
        generation++
    }

    fun copyViewport(x0: Int, y0: Int, x1: Int, y1: Int): String = synchronized(lock) {
        engine.copyViewport(x0, y0, x1, y1)
    }

    fun encodePointer(col: Int, row: Int, button: Int, pressed: Boolean, moving: Boolean): ByteArray? =
        synchronized(lock) {
            if (!mouseTracking) null else engine.encodePointer(col, row, button, pressed, moving)
        }

    fun encodePaste(text: String): ByteArray = synchronized(lock) {
        engine.encodePaste(text)
    }

    override fun close() {
        engine.close()
    }

    private companion object {
        const val PACKED_CELL_BYTES = 20

        fun unpackCell(buf: ByteBuffer): Cell {
            val cp = buf.int
            val fg = buf.int
            val bg = buf.int
            val fgIndex = buf.short.toInt()
            val bgIndex = buf.short.toInt()
            val attrs = buf.short.toInt() and 0xFFFF
            val width = buf.get().toInt() and 0xFF
            buf.get() // reserved padding byte
            return Cell(
                codepoint = cp,
                fg = fg,
                bg = bg,
                attrs = attrs,
                width = width,
                fgIndex = fgIndex,
                bgIndex = bgIndex,
            )
        }
    }
}

data class Cell(
    val codepoint: Int,
    val fg: Int,
    val bg: Int,
    val attrs: Int,
    val width: Int,
    /** -1 默认色，-2 真彩色（用 fg/bg 里的 RGB），0–255 调色板下标。 */
    val fgIndex: Int = Color.INDEX_DEFAULT,
    val bgIndex: Int = Color.INDEX_DEFAULT,
) {
    fun glyphText(): String {
        if (codepoint <= 0) return " "
        if (codepoint <= 0xFFFF) return codepoint.toChar().toString()
        return if (Character.isValidCodePoint(codepoint)) String(Character.toChars(codepoint)) else " "
    }

    companion object {
        fun blank() = Cell(' '.code, Color.DEFAULT_FG, Color.DEFAULT_BG, 0, 1)
    }
}

object Attr {
    const val BOLD = 1
    const val DIM = 2
    const val ITALIC = 4
    const val UNDERLINE = 8
    const val INVERSE = 16
    const val HIDDEN = 32
    const val STRIKE = 64
}

object Color {
    const val INDEX_DEFAULT = -1
    const val INDEX_RGB = -2
    const val DEFAULT_FG = 0x00C0CAF5.toInt()
    const val DEFAULT_BG = 0x001A1B26
    private val ANSI = intArrayOf(
        0x00151520, 0x00F7768E, 0x009ECE6A, 0x00E0AF68,
        0x007AA2F7, 0x00BB9AF7, 0x007DCFFF, 0x00C0CAF5,
        0x00565F89, 0x00F7768E, 0x009ECE6A, 0x00E0AF68,
        0x007AA2F7, 0x00BB9AF7, 0x007DCFFF, 0x00C0CAF5,
    )

    fun ansi(index: Int, bright: Boolean): Int {
        val i = index.coerceIn(0, 7) + if (bright) 8 else 0
        return ANSI[i]
    }

    fun indexed(index: Int): Int {
        val i = index.coerceIn(0, 255)
        if (i < 16) return ANSI[i]
        if (i < 232) {
            val v = i - 16
            val r = v / 36
            val g = (v % 36) / 6
            val b = v % 6
            fun level(n: Int) = if (n == 0) 0 else 55 + n * 40
            return rgb(level(r), level(g), level(b))
        }
        val g = 8 + (i - 232) * 10
        return rgb(g, g, g)
    }

    fun rgb(r: Int, g: Int, b: Int): Int {
        return ((r and 0xff) shl 16) or ((g and 0xff) shl 8) or (b and 0xff)
    }
}
