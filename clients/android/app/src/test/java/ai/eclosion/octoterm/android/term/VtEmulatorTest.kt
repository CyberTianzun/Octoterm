package ai.eclosion.octoterm.android.term

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class VtEmulatorTest {
    @Test
    fun printsPlainText() {
        val t = VtEmulator(20, 5)
        t.write("hi".toByteArray())
        assertEquals('h', t.cell(0, 0).ch)
        assertEquals('i', t.cell(1, 0).ch)
        assertEquals(2, t.cursorX)
    }

    @Test
    fun cupAndColor() {
        val t = VtEmulator(20, 5)
        t.write("\u001b[2;3H\u001b[31mX".toByteArray())
        assertEquals(1, t.cursorY)
        assertEquals(3, t.cursorX)
        assertEquals('X', t.cell(2, 1).ch)
        assertEquals(Color.ansi(1, false), t.cell(2, 1).fg)
    }

    @Test
    fun resetClearsGrid() {
        val t = VtEmulator(20, 5)
        t.write("abc".toByteArray())
        t.reset()
        assertEquals(' ', t.cell(0, 0).ch)
        assertEquals(0, t.cursorX)
        assertEquals(0, t.cursorY)
    }

    @Test
    fun altScreenAndCursorHide() {
        val t = VtEmulator(20, 5)
        t.write("keep".toByteArray())
        t.write("\u001b[?1049h\u001b[2J\u001b[Hvim".toByteArray())
        assertEquals('v', t.cell(0, 0).ch)
        t.write("\u001b[?25l".toByteArray())
        assertFalse(t.cursorVisible)
        t.write("\u001b[?1049l".toByteArray())
        assertEquals('k', t.cell(0, 0).ch)
    }

    @Test
    fun wrapsAtMarginLikeXterm() {
        val t = VtEmulator(4, 3)
        t.write("abcde".toByteArray())
        assertEquals("abcd", (0 until 4).map { t.cell(it, 0).ch }.joinToString(""))
        assertEquals('e', t.cell(0, 1).ch)
        assertEquals(1, t.cursorX)
        assertEquals(1, t.cursorY)
    }

    @Test
    fun decawmOffOverwritesLastCell() {
        val t = VtEmulator(4, 3)
        t.write("\u001b[?7labcde".toByteArray())
        assertEquals("abce", (0 until 4).map { t.cell(it, 0).ch }.joinToString(""))
        assertEquals(' ', t.cell(0, 1).ch)
        assertEquals(3, t.cursorX)
        assertEquals(0, t.cursorY)
    }

    @Test
    fun carriageReturnCancelsPendingWrap() {
        val t = VtEmulator(4, 3)
        t.write("abcd\rX".toByteArray())
        assertEquals('X', t.cell(0, 0).ch)
        assertEquals('b', t.cell(1, 0).ch)
        assertEquals(' ', t.cell(0, 1).ch)
    }

    @Test
    fun resizeKeepsContentWidth() {
        val t = VtEmulator(10, 3)
        t.write("hello".toByteArray())
        t.resize(20, 8)
        assertEquals(20, t.cols)
        assertEquals(8, t.rows)
        assertEquals('h', t.cell(0, 0).ch)
    }
}
