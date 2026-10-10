package ai.eclosion.octoterm.android.term

/** 把触点和位移换成格子。画布和手势共用这一套，避免选区和画面差一格。 */
object TermGeometry {
    fun origin(width: Float, height: Float, cols: Int, rows: Int, cellW: Float, cellH: Float): Pair<Float, Float> {
        if (cols <= 0 || rows <= 0 || cellW <= 0f || cellH <= 0f) return 0f to 0f
        val ox = ((width - cols * cellW) / 2f).coerceAtLeast(0f)
        val oy = ((height - rows * cellH) / 2f).coerceAtLeast(0f)
        return ox to oy
    }

    fun cellAt(
        x: Float,
        y: Float,
        ox: Float,
        oy: Float,
        cellW: Float,
        cellH: Float,
        cols: Int,
        rows: Int,
    ): Pair<Int, Int>? {
        if (cellW <= 0f || cellH <= 0f || cols <= 0 || rows <= 0) return null
        val col = ((x - ox) / cellW).toInt()
        val row = ((y - oy) / cellH).toInt()
        if (col !in 0 until cols || row !in 0 until rows) return null
        return col to row
    }

    /**
     * 手指往下拖是正 dy，视口跟着往下，露出更老的行。
     * 返回剩余的亚像素累积，以及这次该翻的整行数（可负）。
     */
    fun scrollDelta(accum: Float, dy: Float, cellH: Float): Pair<Float, Int> {
        if (cellH <= 0f) return accum to 0
        val next = accum + dy
        val lines = (next / cellH).toInt()
        return (next - lines * cellH) to lines
    }
}

data class TermSelection(
    val x0: Int,
    val y0: Int,
    val x1: Int,
    val y1: Int,
)
