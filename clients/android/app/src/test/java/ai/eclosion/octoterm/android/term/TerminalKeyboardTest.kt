package ai.eclosion.octoterm.android.term

import android.view.KeyEvent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TerminalKeyboardTest {
    private fun ByteArray.text() = toString(Charsets.UTF_8)

    @Test
    fun windowsShortcutsMatchNativeConptyProbeRecords() {
        val keyboard = TerminalKeyboard("windows")
        val cases = listOf(
            KeyStroke(TerminalKey.Enter, modifiers = KeyModifiers(shift = true)) to "13;28;13;1;16;1_",
            KeyStroke(TerminalKey.J, modifiers = KeyModifiers(ctrl = true)) to "74;36;10;1;8;1_",
            KeyStroke(TerminalKey.Up, modifiers = KeyModifiers(alt = true)) to "38;72;0;1;258;1_",
            KeyStroke(TerminalKey.Tab, modifiers = KeyModifiers(ctrl = true)) to "9;15;9;1;8;1_",
            KeyStroke(TerminalKey.PageUp, modifiers = KeyModifiers(shift = true)) to "33;73;0;1;272;1_",
            KeyStroke(TerminalKey.F1, modifiers = KeyModifiers(ctrl = true, shift = true)) to "112;59;0;1;24;1_",
        )
        cases.forEachIndexed { index, (stroke, expected) ->
            assertEquals("\u001b[$expected", keyboard.keyDown(index.toLong(), stroke).text())
            assertEquals("\u001b[${expected.replace(";1;", ";0;")}", keyboard.keyUp(index.toLong(), stroke.modifiers)!!.text())
        }
    }

    @Test
    fun modifierCombinationsKeepFunctionalKeyIdentity() {
        val keys = listOf(TerminalKey.Enter, TerminalKey.Tab, TerminalKey.Up, TerminalKey.Home,
            TerminalKey.End, TerminalKey.Delete, TerminalKey.F12)
        for (key in keys) for (mask in 0..7) {
            val modifiers = KeyModifiers(shift = mask and 1 != 0, ctrl = mask and 2 != 0, alt = mask and 4 != 0)
            val parts = TerminalKeyboard("windows").keyDown(1, KeyStroke(key, modifiers = modifiers))
                .text().removePrefix("\u001b[").removeSuffix("_").split(';').map(String::toInt)
            assertEquals(key.vk, parts[0])
            assertEquals(key.scan, parts[1])
            assertEquals((if (key.enhanced) 256 else 0) or
                (if (modifiers.shift) 16 else 0) or (if (modifiers.ctrl) 8 else 0) or (if (modifiers.alt) 2 else 0), parts[4])
        }
    }

    @Test
    fun actualKeyUpKeepsControlCharacterAfterCtrlWasReleased() {
        val keyboard = TerminalKeyboard("windows")
        assertEquals("\u001b[74;36;10;1;8;1_", keyboard.keyDown(1, KeyStroke(TerminalKey.J, modifiers = KeyModifiers(ctrl = true))).text())
        assertEquals("\u001b[74;36;10;0;0;1_", keyboard.keyUp(1, KeyModifiers())!!.text())
        assertNull(keyboard.keyUp(1, KeyModifiers()))
    }

    @Test
    fun repeatsSendOnlyDownUntilTheRealUp() {
        val keyboard = TerminalKeyboard("windows")
        repeat(3) { assertEquals("\u001b[65;30;97;1;0;1_", keyboard.keyDown(1, KeyStroke(TerminalKey.A)).text()) }
        assertEquals("\u001b[65;30;97;0;0;1_", keyboard.keyUp(1, KeyModifiers())!!.text())
        assertEquals("", keyboard.releaseAll().text())
    }

    @Test
    fun focusLossReleasesPressedKeysAndDisconnectResetSendsNothing() {
        val keyboard = TerminalKeyboard("windows")
        keyboard.keyDown(1, KeyStroke(TerminalKey.AltLeft, modifiers = KeyModifiers(alt = true)))
        keyboard.keyDown(2, KeyStroke(TerminalKey.Up, modifiers = KeyModifiers(alt = true)))
        assertEquals("\u001b[18;56;0;0;0;1_\u001b[38;72;0;0;256;1_", keyboard.releaseAll().text())
        assertFalse(keyboard.hasPressed(2))
        assertNull(keyboard.keyUp(2, KeyModifiers()))
        keyboard.keyDown(3, KeyStroke(TerminalKey.ControlLeft))
        keyboard.reset()
        assertEquals("", keyboard.releaseAll().text())
    }

    @Test
    fun screenTapSendsPairedRecordsWithoutKeepingKeysPressed() {
        val keyboard = TerminalKeyboard("windows")
        assertEquals("\u001b[67;46;3;1;8;1_\u001b[67;46;3;0;8;1_",
            keyboard.tap(KeyStroke(TerminalKey.C, modifiers = KeyModifiers(ctrl = true))).text())
        assertEquals("", keyboard.releaseAll().text())
    }

    @Test
    fun imeTextIsNeverBracketedPasteAndExplicitCtrlJStillWorks() {
        val emulator = VtEmulator()
        emulator.write("\u001b[?2004h".toByteArray())
        assertTrue(emulator.bracketedPaste)
        val keyboard = TerminalKeyboard("windows")
        assertEquals("j", keyboard.committedText("j").text())
        assertEquals("你好😀", keyboard.committedText("你好😀").text())
        assertEquals("\u001b[74;36;10;1;8;1_\u001b[74;36;10;0;8;1_",
            keyboard.committedText("j", KeyModifiers(ctrl = true)).text())
        assertEquals("\u001b[200~j\u001b[201~", emulator.encodePaste("j").text())
    }

    @Test
    fun softReturnAndShiftReturnRemainDistinctOnWindows() {
        val keyboard = TerminalKeyboard("windows")
        assertEquals("\u001b[13;28;13;1;0;1_\u001b[13;28;13;0;0;1_", keyboard.committedText("\n").text())
        assertEquals("\u001b[13;28;13;1;16;1_\u001b[13;28;13;0;16;1_",
            keyboard.committedText("\n", KeyModifiers(shift = true)).text())
    }

    @Test
    fun nonWindowsPreservesVtModesAndModifiedNavigation() {
        for (os in listOf("linux", "macos", "")) {
            val keyboard = TerminalKeyboard(os)
            assertEquals("\u001b[A", keyboard.tap(KeyStroke(TerminalKey.Up)).text())
            assertEquals("\u001bOA", keyboard.tap(KeyStroke(TerminalKey.Up), applicationCursor = true).text())
            assertEquals("\u001b[1;3A", keyboard.tap(KeyStroke(TerminalKey.Up, modifiers = KeyModifiers(alt = true))).text())
            assertEquals("\u001b[H", keyboard.tap(KeyStroke(TerminalKey.Home)).text())
            assertEquals("\u001b[F", keyboard.tap(KeyStroke(TerminalKey.End)).text())
            assertEquals("\u001b[5~", keyboard.tap(KeyStroke(TerminalKey.PageUp)).text())
            assertEquals("\u001b[Z", keyboard.tap(KeyStroke(TerminalKey.Tab, modifiers = KeyModifiers(shift = true))).text())
            assertEquals("\u0003", keyboard.tap(KeyStroke(TerminalKey.C, modifiers = KeyModifiers(ctrl = true))).text())
            assertEquals("\n", keyboard.committedText("j", KeyModifiers(ctrl = true)).text())
            keyboard.keyDown(1, KeyStroke(TerminalKey.A))
            assertEquals("", keyboard.keyUp(1, KeyModifiers())!!.text())
        }
    }

    @Test
    fun unicodeHardwareTextDoesNotCreateAnOrphanWindowsKeyUp() {
        val keyboard = TerminalKeyboard("windows")
        assertEquals("é", keyboard.keyDown(1, KeyStroke(TerminalKey.E, 'é'.code)).text())
        assertEquals("", keyboard.keyUp(1, KeyModifiers())!!.text())
    }

    @Test
    fun rightModifiersLocksAndLayoutArePreserved() {
        val keyboard = TerminalKeyboard("windows")
        val modifiers = KeyModifiers(ctrl = true, alt = true, leftCtrl = true, rightCtrl = true,
            rightAlt = true, capsLock = true, numLock = true, scrollLock = true)
        assertEquals("\u001b[38;72;0;1;493;1_", keyboard.keyDown(1, KeyStroke(TerminalKey.Up, modifiers = modifiers)).text())
        assertEquals("\u001b[65;16;97;1;0;1_", keyboard.keyDown(2, KeyStroke(TerminalKey.Q, 'a'.code)).text())
    }

    @Test
    fun androidKeyCodesMapToWindowsIdentities() {
        assertEquals(TerminalKey.J, AndroidKeyMapper.key(KeyEvent.KEYCODE_J))
        assertEquals(36, AndroidKeyMapper.key(KeyEvent.KEYCODE_J)!!.scan)
        assertEquals(TerminalKey.Up, AndroidKeyMapper.key(KeyEvent.KEYCODE_DPAD_UP))
        assertEquals(38, AndroidKeyMapper.key(KeyEvent.KEYCODE_DPAD_UP)!!.vk)
        assertEquals(TerminalKey.Home, AndroidKeyMapper.key(KeyEvent.KEYCODE_MOVE_HOME))
        assertEquals(TerminalKey.NumpadEnter, AndroidKeyMapper.key(KeyEvent.KEYCODE_NUMPAD_ENTER))
        assertNull(AndroidKeyMapper.key(KeyEvent.KEYCODE_BACK))
    }
}
