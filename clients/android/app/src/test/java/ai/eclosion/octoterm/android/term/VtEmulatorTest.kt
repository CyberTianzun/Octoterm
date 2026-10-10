package ai.eclosion.octoterm.android.term

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
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

    @Test
    fun scrollbackRevealsOlderLinesAndStaysPutOnNewOutput() {
        val t = VtEmulator(4, 2, maxScrollback = 10)
        t.write("aaaa\r\nbbbb\r\ncccc".toByteArray())
        assertEquals("bbbb", row(t, 0))
        assertEquals("cccc", row(t, 1))
        t.scrollBy(1)
        val frame = TermFrame.capture(t)
        assertEquals(1, frame.scrollOffset)
        assertEquals('a', frame.cells[0][0].ch)
        assertEquals("bbbb", (0 until 4).map { frame.cells[1][it].ch }.joinToString(""))
        assertFalse(frame.cursorVisible)
        t.write("Z".toByteArray())
        assertEquals(1, t.scrollOffset)
        t.scrollToBottom()
        assertEquals(0, t.scrollOffset)
        assertEquals('Z', t.cell(0, 1).ch)
    }

    @Test
    fun copyViewportTrimsAndKeepsWideGlyphOnce() {
        val t = VtEmulator(6, 3)
        t.write("ab\r\nc中".toByteArray())
        assertEquals("ab\nc中", t.copyViewport(0, 0, 2, 1))
    }

    @Test
    fun mouseSgrAndPlain() {
        val t = VtEmulator(10, 5)
        assertNull(t.encodePointer(0, 0, 0, pressed = true, moving = false))
        t.write("\u001b[?1000;1006h".toByteArray())
        assertEquals(
            "\u001b[<0;1;1M",
            t.encodePointer(0, 0, 0, pressed = true, moving = false)!!.toString(Charsets.US_ASCII),
        )
        assertEquals(
            "\u001b[<0;2;3m",
            t.encodePointer(1, 2, 0, pressed = false, moving = false)!!.toString(Charsets.US_ASCII),
        )
        assertNull(t.encodePointer(1, 2, 0, pressed = true, moving = true))
        t.write("\u001b[?1002h\u001b[?1006l".toByteArray())
        val plain = t.encodePointer(0, 0, 0, pressed = true, moving = false)!!
        assertEquals(0x1b, plain[0].toInt() and 0xff)
        assertEquals('M'.code, plain[2].toInt() and 0xff)
        assertEquals(32, plain[3].toInt() and 0xff)
        assertEquals(33, plain[4].toInt() and 0xff)
        assertEquals(33, plain[5].toInt() and 0xff)
    }
}

private fun row(t: VtEmulator, y: Int): String {
    return (0 until t.cols).map { t.cell(it, y).ch }.joinToString("")
}
