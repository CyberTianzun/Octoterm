package ai.eclosion.octoterm.android.launcher

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LaunchersTest {
    @Test
    fun keepsCompleteEntriesAndDropsTheRest() {
        val body = """
            {"launchers":[
              {"id":"builtin:shell","provider":"builtin","name":"zsh","detail":"/bin/zsh","command":["/bin/zsh"],"cwd":null},
              {"id":"","name":"nope","command":["x"]},
              {"id":"config:ssh","provider":"config","name":"prod","detail":"ssh prod","command":["ssh","prod"],"cwd":"~/work"},
              {"id":"bad","name":"bad","command":[]},
              {"id":"bad2","name":"bad2","command":[1]}
            ]}
        """.trimIndent()
        val list = Launchers.sanitize(body)
        assertEquals(listOf("builtin:shell", "config:ssh"), list.map { it.id })
        assertEquals(listOf("ssh", "prod"), list[1].command)
        assertEquals("~/work", list[1].cwd)
    }

    @Test
    fun brokenJsonIsEmpty() {
        assertTrue(Launchers.sanitize("{").isEmpty())
        assertTrue(Launchers.fallback().command.isEmpty())
    }
}
