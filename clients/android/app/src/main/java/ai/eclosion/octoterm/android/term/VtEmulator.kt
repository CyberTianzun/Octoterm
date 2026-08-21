package ai.eclosion.octoterm.android.term

/**
 * 客户端哑 VT 渲染器。接口按「喂字节 / reset / 权威 resize」收，后面可以换成 libghostty。
 */
class VtEmulator(
    cols: Int = 80,
    rows: Int = 24,
    private val maxScrollback: Int = 2000,
) {
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

    @Volatile
    var generation: Long = 0L
        private set

    @Volatile
    var onInvalidate: (() -> Unit)? = null

    private val lock = Any()
    private val primary = Screen(this.cols, this.rows, maxScrollback)
    private val alternate = Screen(this.cols, this.rows, scrollback = 0)
    private var screen = primary

    private var fg: Int = Color.DEFAULT_FG
    private var bg: Int = Color.DEFAULT_BG
    private var attrs: Int = 0
    private var originMode = false
    private var autoWrap = true
    private var wrapPending = false
    private var scrollTop = 0
    private var scrollBottom = this.rows - 1
    private var savedX = 0
    private var savedY = 0

    private var utf8Need = 0
    private var utf8Acc = 0
    private var esc = Ground
    private val params = IntArray(16)
    private var paramCount = 0
    private var paramNeg = false
    private var osc = StringBuilder()
    private var decPrivate = false

    fun snapshot(): Array<Array<Cell>> {
        synchronized(lock) {
            return Array(rows) { y -> Array(cols) { x -> screen.cell(x, y).copy() } }
        }
    }

    fun cell(x: Int, y: Int): Cell = synchronized(lock) { screen.cell(x, y) }

    fun reset() {
        synchronized(lock) {
            primary.reset(cols, rows)
            alternate.reset(cols, rows)
            screen = primary
            cursorX = 0
            cursorY = 0
            cursorVisible = true
            applicationCursor = false
            bracketedPaste = false
            fg = Color.DEFAULT_FG
            bg = Color.DEFAULT_BG
            attrs = 0
            originMode = false
            autoWrap = true
            wrapPending = false
            scrollTop = 0
            scrollBottom = rows - 1
            utf8Need = 0
            esc = Ground
            paramCount = 0
            decPrivate = false
            osc.setLength(0)
            generation++
        }
        notifyChanged()
    }

    fun resize(newCols: Int, newRows: Int) {
        val c = newCols.coerceAtLeast(1)
        val r = newRows.coerceAtLeast(1)
        synchronized(lock) {
            if (c == cols && r == rows) return
            cols = c
            rows = r
            primary.resize(c, r)
            alternate.resize(c, r)
            cursorX = cursorX.coerceIn(0, cols - 1)
            cursorY = cursorY.coerceIn(0, rows - 1)
            wrapPending = false
            scrollTop = 0
            scrollBottom = rows - 1
            generation++
        }
        notifyChanged()
    }

    fun write(bytes: ByteArray) {
        synchronized(lock) {
            for (b in bytes) consume(b.toInt() and 0xff)
            generation++
        }
        notifyChanged()
    }

    fun encodePaste(text: String): ByteArray {
        val raw = text.replace("\n", "\r")
        return if (bracketedPaste) {
            ("\u001b[200~$raw\u001b[201~").toByteArray(Charsets.UTF_8)
        } else {
            raw.toByteArray(Charsets.UTF_8)
        }
    }

    fun encodeArrow(dx: Int, dy: Int): ByteArray {
        val letter = when {
            dy < 0 -> if (applicationCursor) "A" else "A"
            dy > 0 -> "B"
            dx > 0 -> "C"
            else -> "D"
        }
        val seq = if (applicationCursor) "\u001bO$letter" else "\u001b[$letter"
        return seq.toByteArray(Charsets.US_ASCII)
    }

    private fun consume(b: Int) {
        if (utf8Need > 0) {
            if (b and 0xc0 == 0x80) {
                utf8Acc = (utf8Acc shl 6) or (b and 0x3f)
                utf8Need--
                if (utf8Need == 0) printCp(utf8Acc)
            } else {
                utf8Need = 0
                printCp(0xfffd)
                consume(b)
            }
            return
        }
        when (esc) {
            Ground -> ground(b)
            Esc -> esc(b)
            Csi -> csi(b)
            Osc -> osc(b)
            IgnoreSt -> if (b == 0x07 || b == 0x5c) esc = Ground
        }
    }

    private fun ground(b: Int) {
        when {
            b == 0x1b -> esc = Esc
            b == 0x07 -> Unit
            b == 0x08 -> {
                if (wrapPending) wrapPending = false
                else cursorX = (cursorX - 1).coerceAtLeast(0)
            }
            b == 0x09 -> {
                wrapPending = false
                cursorX = ((cursorX + 8) / 8 * 8).coerceAtMost(cols - 1)
            }
            b == 0x0a || b == 0x0b || b == 0x0c -> {
                wrapPending = false
                lineFeed()
            }
            b == 0x0d -> {
                wrapPending = false
                cursorX = 0
            }
            b == 0x7f -> Unit
            b < 0x20 -> Unit
            b < 0x80 -> printCp(b)
            b and 0xe0 == 0xc0 -> startUtf8(b, 1, b and 0x1f)
            b and 0xf0 == 0xe0 -> startUtf8(b, 2, b and 0x0f)
            b and 0xf8 == 0xf0 -> startUtf8(b, 3, b and 0x07)
            else -> printCp(0xfffd)
        }
    }

    private fun startUtf8(b: Int, more: Int, acc: Int) {
        utf8Need = more
        utf8Acc = acc
    }

    private fun esc(b: Int) {
        when (b.toChar()) {
            '[' -> {
                esc = Csi
                paramCount = 0
                params[0] = 0
                paramNeg = false
                decPrivate = false
            }
            ']' -> {
                esc = Osc
                osc.setLength(0)
            }
            'P', 'X', '^', '_' -> esc = IgnoreSt
            'c' -> reset()
            '7' -> {
                savedX = cursorX
                savedY = cursorY
                esc = Ground
            }
            '8' -> {
                wrapPending = false
                cursorX = savedX.coerceIn(0, cols - 1)
                cursorY = savedY.coerceIn(0, rows - 1)
                esc = Ground
            }
            'D' -> {
                wrapPending = false
                lineFeed()
                esc = Ground
            }
            'M' -> {
                wrapPending = false
                reverseIndex()
                esc = Ground
            }
            'E' -> {
                wrapPending = false
                cursorX = 0
                lineFeed()
                esc = Ground
            }
            '(', ')', '*', '+' -> Unit
            else -> esc = Ground
        }
    }

    private fun osc(b: Int) {
        if (b == 0x07 || b == 0x5c) {
            esc = Ground
        } else if (b == 0x1b) {
            esc = IgnoreSt
        } else if (osc.length < 1024) {
            osc.append(b.toChar())
        }
    }

    private fun csi(b: Int) {
        when (b.toChar()) {
            '?' -> decPrivate = true
            ';' -> {
                if (paramCount < params.lastIndex) {
                    paramCount++
                    params[paramCount] = 0
                }
            }
            in '0'..'9' -> {
                if (paramCount == 0 && params[0] == 0 && paramCount == 0) {
                    paramCount = 0
                }
                val idx = paramCount
                params[idx] = params[idx] * 10 + (b - '0'.code)
            }
            else -> {
                if (paramCount == 0 && params[0] == 0) {
                    // zero params: treat as empty
                } else if (params[paramCount] != 0 || paramCount > 0) {
                    paramCount++
                }
                execCsi(b.toChar())
                esc = Ground
            }
        }
    }

    private fun p(i: Int, default: Int = 1): Int {
        val v = if (i < paramCount) params[i] else if (i == 0 && paramCount == 0) 0 else 0
        return if (v == 0) default else v
    }

    private fun execCsi(cmd: Char) {
        when (cmd) {
            'A' -> {
                wrapPending = false
                cursorY = (cursorY - p(0)).coerceAtLeast(minY())
            }
            'B' -> {
                wrapPending = false
                cursorY = (cursorY + p(0)).coerceAtMost(maxY())
            }
            'C' -> {
                wrapPending = false
                cursorX = (cursorX + p(0)).coerceAtMost(cols - 1)
            }
            'D' -> {
                wrapPending = false
                cursorX = (cursorX - p(0)).coerceAtLeast(0)
            }
            'H', 'f' -> {
                wrapPending = false
                val y = p(0, 1) - 1
                val x = p(1, 1) - 1
                cursorY = (y + minY()).coerceIn(minY(), maxY())
                cursorX = x.coerceIn(0, cols - 1)
            }
            'G' -> {
                wrapPending = false
                cursorX = (p(0, 1) - 1).coerceIn(0, cols - 1)
            }
            'd' -> {
                wrapPending = false
                cursorY = (p(0, 1) - 1).coerceIn(0, rows - 1)
            }
            'J' -> eraseDisplay(if (paramCount == 0) 0 else params[0])
            'K' -> eraseLine(if (paramCount == 0) 0 else params[0])
            'm' -> sgr()
            'r' -> {
                val top = (p(0, 1) - 1).coerceIn(0, rows - 1)
                val bot = (if (paramCount > 1) params[1] else rows) - 1
                scrollTop = top
                scrollBottom = bot.coerceIn(top, rows - 1)
            }
            'h' -> setMode(true)
            'l' -> setMode(false)
            'n' -> Unit
            '@' -> insertChars(p(0))
            'P' -> deleteChars(p(0))
            'L' -> insertLines(p(0))
            'M' -> deleteLines(p(0))
            'S' -> repeat(p(0)) { scrollUp() }
            'T' -> repeat(p(0)) { scrollDown() }
        }
    }

    private fun setMode(enable: Boolean) {
        if (!decPrivate) return
        val code = if (paramCount == 0) params[0] else params[0]
        when (code) {
            1 -> applicationCursor = enable
            7 -> {
                autoWrap = enable
                if (!enable) wrapPending = false
            }
            25 -> cursorVisible = enable
            2004 -> bracketedPaste = enable
            47, 1047, 1049 -> {
                screen = if (enable) {
                    if (code == 1049) {
                        savedX = cursorX
                        savedY = cursorY
                    }
                    alternate.reset(cols, rows)
                    alternate
                } else {
                    if (code == 1049) {
                        cursorX = savedX.coerceIn(0, cols - 1)
                        cursorY = savedY.coerceIn(0, rows - 1)
                    }
                    primary
                }
            }
        }
    }

    private fun sgr() {
        if (paramCount == 0) {
            fg = Color.DEFAULT_FG
            bg = Color.DEFAULT_BG
            attrs = 0
            return
        }
        var i = 0
        while (i < paramCount) {
            when (val n = params[i]) {
                0 -> {
                    fg = Color.DEFAULT_FG
                    bg = Color.DEFAULT_BG
                    attrs = 0
                }
                1 -> attrs = attrs or Attr.BOLD
                2 -> attrs = attrs or Attr.DIM
                3 -> attrs = attrs or Attr.ITALIC
                4 -> attrs = attrs or Attr.UNDERLINE
                7 -> attrs = attrs or Attr.INVERSE
                8 -> attrs = attrs or Attr.HIDDEN
                9 -> attrs = attrs or Attr.STRIKE
                21, 22 -> attrs = attrs and (Attr.BOLD or Attr.DIM).inv()
                23 -> attrs = attrs and Attr.ITALIC.inv()
                24 -> attrs = attrs and Attr.UNDERLINE.inv()
                27 -> attrs = attrs and Attr.INVERSE.inv()
                28 -> attrs = attrs and Attr.HIDDEN.inv()
                29 -> attrs = attrs and Attr.STRIKE.inv()
                in 30..37 -> fg = Color.ansi(n - 30, bright = false)
                38 -> i += applyExt(i, fg = true)
                39 -> fg = Color.DEFAULT_FG
                in 40..47 -> bg = Color.ansi(n - 40, bright = false)
                48 -> i += applyExt(i, fg = false)
                49 -> bg = Color.DEFAULT_BG
                in 90..97 -> fg = Color.ansi(n - 90, bright = true)
                in 100..107 -> bg = Color.ansi(n - 100, bright = true)
            }
            i++
        }
    }

    private fun applyExt(i: Int, fg: Boolean): Int {
        if (i + 1 >= paramCount) return 0
        return when (params[i + 1]) {
            5 -> {
                if (i + 2 < paramCount) {
                    val c = Color.indexed(params[i + 2])
                    if (fg) this.fg = c else bg = c
                }
                2
            }
            2 -> {
                if (i + 4 < paramCount) {
                    val c = Color.rgb(params[i + 2], params[i + 3], params[i + 4])
                    if (fg) this.fg = c else bg = c
                }
                4
            }
            else -> 1
        }
    }

    private fun printCp(cp: Int) {
        val ch = if (Character.isValidCodePoint(cp)) String(Character.toChars(cp))[0] else ' '
        val w = glyphWidth(cp)
        if (wrapPending && autoWrap) {
            wrapPending = false
            cursorX = 0
            lineFeed()
        }
        if (cursorX + w > cols) {
            if (autoWrap) {
                cursorX = 0
                lineFeed()
            } else {
                cursorX = (cols - w).coerceAtLeast(0)
            }
        }
        screen.set(cursorX, cursorY, Cell(ch, fg, bg, attrs, w))
        if (w == 2 && cursorX + 1 < cols) {
            screen.set(cursorX + 1, cursorY, Cell(' ', fg, bg, attrs, 0))
        }
        cursorX += w
        if (cursorX >= cols) {
            cursorX = cols - 1
            wrapPending = autoWrap
        }
    }

    private fun lineFeed() {
        if (cursorY == scrollBottom) {
            scrollUp()
        } else if (cursorY < rows - 1) {
            cursorY++
        }
    }

    private fun reverseIndex() {
        if (cursorY == scrollTop) scrollDown() else if (cursorY > 0) cursorY--
    }

    private fun scrollUp() {
        screen.scrollUp(scrollTop, scrollBottom)
    }

    private fun scrollDown() {
        screen.scrollDown(scrollTop, scrollBottom)
    }

    private fun eraseDisplay(mode: Int) {
        when (mode) {
            0 -> {
                eraseLine(0)
                for (y in cursorY + 1 until rows) screen.clearRow(y)
            }
            1 -> {
                for (y in 0 until cursorY) screen.clearRow(y)
                eraseLine(1)
            }
            else -> {
                for (y in 0 until rows) screen.clearRow(y)
                if (mode == 2 || mode == 3) {
                    cursorX = 0
                    cursorY = 0
                }
            }
        }
    }

    private fun eraseLine(mode: Int) {
        when (mode) {
            0 -> for (x in cursorX until cols) screen.set(x, cursorY, Cell.blank())
            1 -> for (x in 0..cursorX) screen.set(x, cursorY, Cell.blank())
            else -> screen.clearRow(cursorY)
        }
    }

    private fun insertChars(n: Int) {
        screen.insertCells(cursorX, cursorY, n)
    }

    private fun deleteChars(n: Int) {
        screen.deleteCells(cursorX, cursorY, n)
    }

    private fun insertLines(n: Int) {
        repeat(n) { screen.scrollDown(cursorY.coerceAtLeast(scrollTop), scrollBottom) }
    }

    private fun deleteLines(n: Int) {
        repeat(n) { screen.scrollUp(cursorY.coerceAtLeast(scrollTop), scrollBottom) }
    }

    private fun minY() = if (originMode) scrollTop else 0
    private fun maxY() = if (originMode) scrollBottom else rows - 1

    private fun notifyChanged() {
        onInvalidate?.invoke()
    }

    private companion object {
        const val Ground = 0
        const val Esc = 1
        const val Csi = 2
        const val Osc = 3
        const val IgnoreSt = 4
    }
}

data class Cell(
    val ch: Char,
    val fg: Int,
    val bg: Int,
    val attrs: Int,
    val width: Int,
) {
    companion object {
        fun blank() = Cell(' ', Color.DEFAULT_FG, Color.DEFAULT_BG, 0, 1)
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

private class Screen(
    private var cols: Int,
    private var rows: Int,
    private val scrollback: Int,
) {
    private val lines = ArrayDeque<Array<Cell>>()

    init {
        reset(cols, rows)
    }

    fun reset(c: Int, r: Int) {
        cols = c
        rows = r
        lines.clear()
        repeat(rows) { lines.add(blankRow()) }
    }

    fun resize(c: Int, r: Int) {
        cols = c
        rows = r
        while (lines.size < rows) lines.add(blankRow())
        while (lines.size > rows + scrollback) lines.removeFirst()
        for (i in lines.indices) {
            val row = lines[i]
            if (row.size != cols) {
                lines[i] = Array(cols) { x -> if (x < row.size) row[x] else Cell.blank() }
            }
        }
        while (visibleStart() + rows > lines.size) lines.add(blankRow())
    }

    fun cell(x: Int, y: Int): Cell {
        val row = lines.getOrNull(visibleStart() + y) ?: return Cell.blank()
        return row.getOrElse(x) { Cell.blank() }
    }

    fun set(x: Int, y: Int, cell: Cell) {
        if (x !in 0 until cols || y !in 0 until rows) return
        val idx = visibleStart() + y
        ensure(idx)
        lines[idx][x] = cell
    }

    fun clearRow(y: Int) {
        if (y !in 0 until rows) return
        val idx = visibleStart() + y
        ensure(idx)
        lines[idx] = blankRow()
    }

    fun scrollUp(top: Int, bottom: Int) {
        if (scrollback > 0 && top == 0 && bottom == rows - 1) {
            lines.add(blankRow())
            trim()
            return
        }
        val t = (visibleStart() + top).coerceAtLeast(0)
        val b = (visibleStart() + bottom).coerceAtMost(lines.size - 1)
        if (t > b) return
        lines.removeAt(t)
        lines.add(b, blankRow())
    }

    fun scrollDown(top: Int, bottom: Int) {
        val t = visibleStart() + top
        val b = visibleStart() + bottom
        if (t > b || t < 0 || b >= lines.size) return
        lines.add(t, blankRow())
        if (b + 1 < lines.size) lines.removeAt(b + 1)
    }

    fun insertCells(x: Int, y: Int, n: Int) {
        val idx = visibleStart() + y
        if (idx !in lines.indices || x !in 0 until cols) return
        val row = lines[idx]
        val out = Array(cols) { Cell.blank() }
        var j = 0
        for (i in 0 until x) out[j++] = row[i]
        repeat(n.coerceAtMost(cols - x)) { j++ }
        var i = x
        while (j < cols && i < cols) out[j++] = row[i++]
        lines[idx] = out
    }

    fun deleteCells(x: Int, y: Int, n: Int) {
        val idx = visibleStart() + y
        if (idx !in lines.indices || x !in 0 until cols) return
        val row = lines[idx]
        val out = Array(cols) { Cell.blank() }
        var j = 0
        for (i in 0 until x) out[j++] = row[i]
        var i = x + n
        while (j < cols && i < cols) out[j++] = row[i++]
        lines[idx] = out
    }

    private fun visibleStart(): Int = (lines.size - rows).coerceAtLeast(0)

    private fun ensure(idx: Int) {
        while (lines.size <= idx) lines.add(blankRow())
    }

    private fun trim() {
        val max = rows + scrollback
        while (lines.size > max) lines.removeFirst()
    }

    private fun blankRow() = Array(cols) { Cell.blank() }
}

private fun glyphWidth(cp: Int): Int {
    if (cp == 0) return 0
    if (cp < 0x20 || cp == 0x7f) return 0
    if (cp < 0x1100) return 1
    return if (
        cp in 0x1100..0x115F ||
        cp in 0x2E80..0xA4CF ||
        cp in 0xAC00..0xD7A3 ||
        cp in 0xF900..0xFAFF ||
        cp in 0xFE10..0xFE6F ||
        cp in 0xFF00..0xFF60 ||
        cp in 0xFFE0..0xFFE6 ||
        cp in 0x1F300..0x1FAFF
    ) 2 else 1
}
