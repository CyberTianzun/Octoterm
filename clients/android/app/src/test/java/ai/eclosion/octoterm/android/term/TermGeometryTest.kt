package ai.eclosion.octoterm.android.term

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TermGeometryTest {
    @Test
    fun cellAtUsesTheCenteredOrigin() {
        val (ox, oy) = TermGeometry.origin(100f, 40f, cols = 4, rows = 2, cellW = 10f, cellH = 10f)
        assertEquals(30f, ox)
        assertEquals(10f, oy)
        assertEquals(0 to 0, TermGeometry.cellAt(30f, 10f, ox, oy, 10f, 10f, 4, 2))
        assertEquals(3 to 1, TermGeometry.cellAt(69f, 29f, ox, oy, 10f, 10f, 4, 2))
        assertNull(TermGeometry.cellAt(0f, 0f, ox, oy, 10f, 10f, 4, 2))
    }

    @Test
    fun scrollDeltaKeepsTheSubpixelRemainder() {
        val (accum, lines) = TermGeometry.scrollDelta(3f, 12f, cellH = 10f)
        assertEquals(1, lines)
        assertEquals(5f, accum)
    }
}
