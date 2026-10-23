package ai.eclosion.octoterm.android.connection

import ai.eclosion.octoterm.android.agent.AgentSession
import ai.eclosion.octoterm.android.agent.AgentSessions
import ai.eclosion.octoterm.android.agent.AgentState
import org.junit.Assert.assertEquals
import org.junit.Test

class AgentActivityTest {
    private fun agent(state: AgentState, id: String = "same-id") =
        AgentSession("codex", id, 1, state, null, null, true)

    @Test
    fun countsAllConnectedHostsWithoutCollapsingTheirIdenticalSessionKeys() {
        val counts = connectedAgentCounts(listOf(
            ServerConnectionStatus.Connected to listOf(agent(AgentState.Working)),
            ServerConnectionStatus.Connected to listOf(agent(AgentState.Working), agent(AgentState.Thinking, "thinking")),
        ))
        assertEquals(mapOf(AgentState.Working to 2, AgentState.Thinking to 1), counts)
    }

    @Test
    fun reconnectingAndDisconnectedCachesDoNotContributeStaleActivity() {
        val cached = listOf(agent(AgentState.Working))
        val servers = ServerConnectionStatus.entries.map { it to cached }
        assertEquals(mapOf(AgentState.Working to 1), connectedAgentCounts(servers))
        assertEquals(emptyMap<AgentState, Int>(), connectedAgentCounts(listOf(
            ServerConnectionStatus.Connected to listOf(agent(AgentState.Done)),
        )))
    }

    @Test
    fun liveEventsAndReplacementSnapshotsRemoveTheLastWorkingCount() {
        var sessions = AgentSessions.replaceAll(listOf(agent(AgentState.Working)))
        fun counts() = connectedAgentCounts(listOf(ServerConnectionStatus.Connected to sessions.values))
        assertEquals(mapOf(AgentState.Working to 1), counts())
        sessions = AgentSessions.applyEvent(sessions, agent(AgentState.Waiting))
        assertEquals(mapOf(AgentState.Waiting to 1), counts())
        sessions = AgentSessions.applyEvent(sessions, agent(AgentState.Done))
        assertEquals(emptyMap<AgentState, Int>(), counts())
        sessions = AgentSessions.replaceAll(listOf(agent(AgentState.Idle)))
        assertEquals(mapOf(AgentState.Idle to 1), counts())
        sessions = AgentSessions.replaceAll(emptyList())
        assertEquals(emptyMap<AgentState, Int>(), counts())
    }
}
