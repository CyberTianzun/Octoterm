package ai.eclosion.octoterm.android.term

/** 一次已提交的终端画面。和终端状态机解耦，reset/resize 不能改正在显示的帧。 */
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

        fun capture(term: AlacrittyTerm): TermFrame {
            val cols = term.cols
            val rows = term.rows
            val cx = term.cursorX
            val cy = term.cursorY + term.scrollOffset
            val visible = term.cursorVisible && cy in 0 until rows && cx in 0 until cols
            return TermFrame(
                cells = term.snapshot(),
                cols = cols,
                rows = rows,
                cursorX = cx,
                cursorY = if (visible) cy else term.cursorY,
                cursorVisible = visible,
                scrollOffset = term.scrollOffset,
            )
        }
    }

    fun hasContent(): Boolean {
        if (cols <= 0 || rows <= 0) return false
        for (row in cells) {
            for (cell in row) {
                if (cell.codepoint != ' '.code && cell.codepoint != 0) return true
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
