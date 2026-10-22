package ai.eclosion.octoterm.android.connection

import org.junit.Assert.*
import org.junit.Test

class ServerConnectionRegistryTest {
    private class Link { var closes = 0 }
    private fun registry() = ServerConnectionRegistry<Link> { it.closes++ }

    @Test
    fun returningToTheServerListKeepsEveryConnectionAlive() {
        val links = registry()
        val first = Link(); val second = Link()
        links["first"] = first; links["second"] = second
        links.select("first"); links.select(null)
        assertEquals(setOf("first", "second"), links.activeIds)
        assertEquals(0, first.closes); assertEquals(0, second.closes)
    }

    @Test
    fun switchingServersDoesNotDisconnectThePreviousOne() {
        val links = registry()
        val first = Link(); val second = Link()
        links["first"] = first; links["second"] = second
        links.select("first"); links.select("second")
        assertEquals("second", links.selectedId)
        assertTrue(links.isCurrent("first", first)); assertTrue(links.isCurrent("second", second))
        assertEquals(0, first.closes)
    }

    @Test
    fun manualDisconnectClosesOnlyThatServerAndRejectsLateCallbacks() {
        val links = registry()
        val first = Link(); val second = Link()
        links["first"] = first; links["second"] = second
        links.select("first"); links.disconnect("first")
        assertNull(links.selectedId)
        assertFalse(links.isCurrent("first", first)); assertTrue(links.isCurrent("second", second))
        assertEquals(1, first.closes); assertEquals(0, second.closes)
        assertSame(first, links["first"])
        links.disconnect("first")
        assertEquals(1, first.closes)
    }

    @Test
    fun reconnectReplacesOnlyTheChosenTransport() {
        val links = registry()
        val first = Link(); val second = Link(); val replacement = Link()
        links["first"] = first; links["second"] = second
        links["first"] = replacement
        assertEquals(1, first.closes); assertEquals(0, second.closes)
        assertFalse(links.isCurrent("first", first)); assertTrue(links.isCurrent("first", replacement))
        links["first"] = replacement
        assertEquals(0, replacement.closes)
    }

    @Test
    fun disconnectAllIsIdempotentAndLeavesNoReconnectDesired() {
        val links = registry()
        val first = Link(); val second = Link()
        links["first"] = first; links["second"] = second
        links.select("first"); links.disconnectAll(); links.disconnectAll()
        assertTrue(links.activeIds.isEmpty()); assertNull(links.selectedId)
        assertEquals(1, first.closes); assertEquals(1, second.closes)
    }
}
