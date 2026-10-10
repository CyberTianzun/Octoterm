package ai.eclosion.octoterm.android.term

/** 一次已提交的终端画面。和模拟器解耦，reset/resize 不能改正在显示的帧。 */
data class TermFrame(
    val cells: Array<Array<Cell>>,
    val cols: Int,
    val rows: Int,
    val cursorX: Int,
    val cursorY: Int,
    val cursorVisible: Boolean,
    val scrollOffset: Int = 0,
) {
    companion object {
        val Empty = TermFrame(
            cells = emptyArray(),
            cols = 0,
            rows = 0,
            cursorX = 0,
            cursorY = 0,
            cursorVisible = false,
        )

        fun capture(emulator: VtEmulator): TermFrame {
            val cursor = emulator.viewportCursor()
            return TermFrame(
                cells = emulator.snapshot(),
                cols = emulator.cols,
                rows = emulator.rows,
                cursorX = cursor?.first ?: emulator.cursorX,
                cursorY = cursor?.second ?: emulator.cursorY,
                cursorVisible = cursor != null,
                scrollOffset = emulator.scrollOffset,
            )
        }
    }

    fun hasContent(): Boolean {
        if (cols <= 0 || rows <= 0) return false
        for (row in cells) {
            for (cell in row) {
                if (cell.ch != ' ' && cell.ch != '\u0000') return true
                if (cell.bg != Color.DEFAULT_BG) return true
            }
        }
        return false
    }

    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is TermFrame) return false
        return cols == other.cols &&
            rows == other.rows &&
            cursorX == other.cursorX &&
            cursorY == other.cursorY &&
            cursorVisible == other.cursorVisible &&
            scrollOffset == other.scrollOffset &&
            cells.contentDeepEquals(other.cells)
    }

    override fun hashCode(): Int {
        var result = cells.contentDeepHashCode()
        result = 31 * result + cols
        result = 31 * result + rows
        result = 31 * result + cursorX
        result = 31 * result + cursorY
        result = 31 * result + cursorVisible.hashCode()
        result = 31 * result + scrollOffset
        return result
    }
}
