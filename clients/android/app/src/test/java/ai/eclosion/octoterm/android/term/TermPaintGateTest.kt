package ai.eclosion.octoterm.android.term

import ai.eclosion.octoterm.android.ui.connections.ConnectionViewModel
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class TermPaintGateTest {
    @Test
    fun openSessionHoldsUntilResyncEnd() {
        val gate = TermPaintGate()
        gate.onSessionOpen()
        assertTrue(gate.holding)
        assertFalse(gate.shouldPublishWrite())
        assertFalse(gate.shouldSendResize())

        gate.onAttached(replay = false)
        gate.onResyncBegin()
        assertFalse(gate.shouldPublishWrite())
        assertFalse(gate.shouldSendResize())

        gate.onResyncEnd()
        assertTrue(gate.shouldPublishWrite())
        assertTrue(gate.shouldSendResize())
    }

    @Test
    fun replayDoesNotHoldTheLastFrame() {
        val gate = TermPaintGate()
        gate.onSessionOpen()
        gate.onAttached(replay = true)
        assertTrue(gate.shouldPublishWrite())
        assertTrue(gate.shouldSendResize())
    }

    @Test
    fun laterResyncHoldsAgainWithoutDroppingResizePermission() {
        val gate = TermPaintGate()
        gate.onSessionOpen()
        gate.onResyncEnd()
        assertTrue(gate.acceptClientResize)

        gate.onResyncBegin()
        assertFalse(gate.shouldPublishWrite())
        assertFalse(gate.shouldSendResize())

        gate.onResyncEnd()
        assertTrue(gate.shouldPublishWrite())
        assertTrue(gate.shouldSendResize())
    }
}

class TermFrameTest {
    @Test
    fun emptyAndClearedHaveNoContent() {
        assertFalse(TermFrame.Empty.hasContent())
        val t = VtEmulator(20, 5)
        t.write("\u001b[0m\u001b[?25l\u001b[2J\u001b[H".toByteArray())
        assertFalse(TermFrame.capture(t).hasContent())
    }

    @Test
    fun printedTextCountsAsContent() {
        val t = VtEmulator(20, 5)
        t.write("hi".toByteArray())
        assertTrue(TermFrame.capture(t).hasContent())
    }
}

class ResizeDeltaTest {
    @Test
    fun ignoresOneCellJitter() {
        assertFalse(ConnectionViewModel.significantResize(90, 30, 89, 30))
        assertFalse(ConnectionViewModel.significantResize(90, 30, 90, 29))
        assertTrue(ConnectionViewModel.significantResize(90, 24, 90, 30))
        assertTrue(ConnectionViewModel.significantResize(80, 30, 90, 30))
    }
}
