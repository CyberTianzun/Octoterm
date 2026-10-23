package ai.eclosion.octoterm.android.connection

import ai.eclosion.octoterm.android.agent.AgentSession
import ai.eclosion.octoterm.android.agent.AgentState

/** Count each server separately: identical agent/session keys can exist on different hosts. */
internal fun connectedAgentCounts(
    servers: Iterable<Pair<ServerConnectionStatus, Collection<AgentSession>>>,
): Map<AgentState, Int> = servers.asSequence()
    .filter { (status, _) -> status == ServerConnectionStatus.Connected }
    .flatMap { (_, agents) -> agents.asSequence() }
    .filter { it.state != AgentState.Done }
    .groupingBy { it.state }
    .eachCount()
