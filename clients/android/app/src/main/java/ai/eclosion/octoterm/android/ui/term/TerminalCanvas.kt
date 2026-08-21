package ai.eclosion.octoterm.android.ui.term

import android.graphics.Typeface
import androidx.compose.foundation.Canvas
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.sp
import ai.eclosion.octoterm.android.term.Attr
import ai.eclosion.octoterm.android.term.TermFrame
import kotlin.math.max

@Composable
fun TerminalCanvas(
    frame: TermFrame,
    onProposeSize: (Int, Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val density = LocalDensity.current
    val textSizePx = with(density) { 14.sp.toPx() }
    val textPaint = remember(textSizePx) {
        android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply {
            typeface = Typeface.MONOSPACE
            textSize = textSizePx
        }
    }
    val bgPaint = remember { android.graphics.Paint() }
    val fm = textPaint.fontMetrics
    val cellW = textPaint.measureText("M")
    val cellH = fm.descent - fm.ascent + 2f
    val baseline = -fm.ascent + 1f
    val shown = frame

    Canvas(
        modifier.onSizeChanged { size ->
            if (cellW <= 0f || cellH <= 0f) return@onSizeChanged
            val cols = (size.width / cellW).toInt()
            val rows = (size.height / cellH).toInt()
            if (cols < 20 || rows < 5) return@onSizeChanged
            onProposeSize(cols, rows)
        },
    ) {
        drawRect(androidx.compose.ui.graphics.Color(0xFF1A1B26))
        val cols = shown.cols
        val rows = shown.rows
        if (cols <= 0 || rows <= 0 || cellW <= 0f || cellH <= 0f) return@Canvas
        val ox = max(0f, (size.width - cols * cellW) / 2f)
        val oy = max(0f, (size.height - rows * cellH) / 2f)
        drawIntoCanvas { canvas ->
            val native = canvas.nativeCanvas
            for (y in 0 until rows) {
                val row = shown.cells.getOrNull(y) ?: continue
                for (x in 0 until cols) {
                    val cell = row.getOrNull(x) ?: continue
                    if (cell.width == 0) continue
                    var fg = cell.fg or 0xFF000000.toInt()
                    var bg = cell.bg or 0xFF000000.toInt()
                    if (cell.attrs and Attr.INVERSE != 0) {
                        val tmp = fg
                        fg = bg
                        bg = tmp
                    }
                    if (cell.attrs and Attr.HIDDEN != 0) fg = bg
                    val left = ox + x * cellW
                    val top = oy + y * cellH
                    bgPaint.color = bg
                    native.drawRect(left, top, left + cellW * cell.width, top + cellH, bgPaint)
                    if (cell.ch != ' ') {
                        textPaint.color = fg
                        textPaint.isFakeBoldText = cell.attrs and Attr.BOLD != 0
                        textPaint.textSkewX = if (cell.attrs and Attr.ITALIC != 0) -0.2f else 0f
                        textPaint.isUnderlineText = cell.attrs and Attr.UNDERLINE != 0
                        native.drawText(cell.ch.toString(), left, top + baseline, textPaint)
                    }
                }
            }
            if (shown.cursorVisible) {
                bgPaint.color = 0xA07AA2F7.toInt()
                val left = ox + shown.cursorX * cellW
                val top = oy + shown.cursorY * cellH
                native.drawRect(left, top, left + cellW, top + cellH, bgPaint)
            }
        }
    }
}
