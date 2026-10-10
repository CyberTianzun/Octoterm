package ai.eclosion.octoterm.android.ui.term

import android.graphics.Typeface
import androidx.compose.foundation.Canvas
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.layout.onSizeChanged
import ai.eclosion.octoterm.android.appearance.TermAppearance
import ai.eclosion.octoterm.android.term.Attr
import ai.eclosion.octoterm.android.term.TermFrame
import ai.eclosion.octoterm.android.term.TermGeometry
import ai.eclosion.octoterm.android.term.TermSelection

class TermPaint(
    val text: android.graphics.Paint,
    val fill: android.graphics.Paint,
    val cellW: Float,
    val cellH: Float,
    val baseline: Float,
)

fun measureTerm(textSizePx: Float, typeface: Typeface): TermPaint {
    val text = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply {
        this.typeface = typeface
        textSize = textSizePx
    }
    val fm = text.fontMetrics
    return TermPaint(
        text = text,
        fill = android.graphics.Paint(),
        cellW = text.measureText("M").coerceAtLeast(1f),
        cellH = (fm.descent - fm.ascent + 2f).coerceAtLeast(1f),
        baseline = -fm.ascent + 1f,
    )
}

@Composable
fun TerminalCanvas(
    frame: TermFrame,
    appearance: TermAppearance,
    selection: TermSelection?,
    paint: TermPaint,
    onProposeSize: (Int, Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    val textPaint = remember(paint) { paint.text }
    val bgPaint = remember(paint) { paint.fill }
    val cellW = paint.cellW
    val cellH = paint.cellH
    val baseline = paint.baseline
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
        drawRect(androidx.compose.ui.graphics.Color(appearance.background or 0xFF000000.toInt()))
        val cols = shown.cols
        val rows = shown.rows
        if (cols <= 0 || rows <= 0) return@Canvas
        val (ox, oy) = TermGeometry.origin(size.width, size.height, cols, rows, cellW, cellH)
        val sel = selection?.let { normalizeSelection(it) }
        drawIntoCanvas { canvas ->
            val native = canvas.nativeCanvas
            for (y in 0 until rows) {
                val row = shown.cells.getOrNull(y) ?: continue
                for (x in 0 until cols) {
                    val cell = row.getOrNull(x) ?: continue
                    if (cell.width == 0) continue
                    var fg = appearance.colorFor(cell.fgIndex, cell.fg, foreground = true)
                    var bg = appearance.colorFor(cell.bgIndex, cell.bg, foreground = false)
                    if (cell.attrs and Attr.INVERSE != 0) {
                        val tmp = fg
                        fg = bg
                        bg = tmp
                    }
                    if (cell.attrs and Attr.HIDDEN != 0) fg = bg
                    val selected = sel != null && inSelection(sel, x, y)
                    val left = ox + x * cellW
                    val top = oy + y * cellH
                    bgPaint.color = if (selected) appearance.selection else (bg or 0xFF000000.toInt())
                    native.drawRect(left, top, left + cellW * cell.width, top + cellH, bgPaint)
                    if (cell.ch != ' ') {
                        textPaint.color = (fg or 0xFF000000.toInt())
                        textPaint.isFakeBoldText = cell.attrs and Attr.BOLD != 0
                        textPaint.textSkewX = if (cell.attrs and Attr.ITALIC != 0) -0.2f else 0f
                        textPaint.isUnderlineText = cell.attrs and Attr.UNDERLINE != 0
                        textPaint.isStrikeThruText = cell.attrs and Attr.STRIKE != 0
                        native.drawText(cell.ch.toString(), left, top + baseline, textPaint)
                    }
                }
            }
            if (shown.cursorVisible && (sel == null)) {
                bgPaint.color = (appearance.cursor or 0xFF000000.toInt()) and 0x00FFFFFF or 0xA0000000.toInt()
                val left = ox + shown.cursorX * cellW
                val top = oy + shown.cursorY * cellH
                native.drawRect(left, top, left + cellW, top + cellH, bgPaint)
            }
        }
    }
}

private fun normalizeSelection(sel: TermSelection): TermSelection {
    return if (sel.y0 < sel.y1 || (sel.y0 == sel.y1 && sel.x0 <= sel.x1)) {
        sel
    } else {
        TermSelection(sel.x1, sel.y1, sel.x0, sel.y0)
    }
}

private fun inSelection(sel: TermSelection, x: Int, y: Int): Boolean {
    if (y < sel.y0 || y > sel.y1) return false
    if (sel.y0 == sel.y1) return x in sel.x0..sel.x1
    if (y == sel.y0) return x >= sel.x0
    if (y == sel.y1) return x <= sel.x1
    return true
}
