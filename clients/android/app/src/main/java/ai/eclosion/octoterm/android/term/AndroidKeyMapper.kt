package ai.eclosion.octoterm.android.term

import android.view.KeyEvent

/** Android scan codes are not Windows Set 1 scan codes: map key identities explicitly. */
object AndroidKeyMapper {
    fun key(code: Int): TerminalKey? = when (code) {
        in KeyEvent.KEYCODE_A..KeyEvent.KEYCODE_Z -> TerminalKey.entries[TerminalKey.A.ordinal + code - KeyEvent.KEYCODE_A]
        in KeyEvent.KEYCODE_0..KeyEvent.KEYCODE_9 -> TerminalKey.entries[TerminalKey.Digit0.ordinal + code - KeyEvent.KEYCODE_0]
        in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F12 -> TerminalKey.entries[TerminalKey.F1.ordinal + code - KeyEvent.KEYCODE_F1]
        in KeyEvent.KEYCODE_NUMPAD_0..KeyEvent.KEYCODE_NUMPAD_9 ->
            TerminalKey.entries[TerminalKey.Numpad0.ordinal + code - KeyEvent.KEYCODE_NUMPAD_0]
        KeyEvent.KEYCODE_ENTER -> TerminalKey.Enter
        KeyEvent.KEYCODE_NUMPAD_ENTER -> TerminalKey.NumpadEnter
        KeyEvent.KEYCODE_DEL -> TerminalKey.Backspace
        KeyEvent.KEYCODE_FORWARD_DEL -> TerminalKey.Delete
        KeyEvent.KEYCODE_ESCAPE -> TerminalKey.Escape
        KeyEvent.KEYCODE_TAB -> TerminalKey.Tab
        KeyEvent.KEYCODE_SPACE -> TerminalKey.Space
        KeyEvent.KEYCODE_DPAD_UP -> TerminalKey.Up
        KeyEvent.KEYCODE_DPAD_DOWN -> TerminalKey.Down
        KeyEvent.KEYCODE_DPAD_LEFT -> TerminalKey.Left
        KeyEvent.KEYCODE_DPAD_RIGHT -> TerminalKey.Right
        KeyEvent.KEYCODE_MOVE_HOME -> TerminalKey.Home
        KeyEvent.KEYCODE_MOVE_END -> TerminalKey.End
        KeyEvent.KEYCODE_PAGE_UP -> TerminalKey.PageUp
        KeyEvent.KEYCODE_PAGE_DOWN -> TerminalKey.PageDown
        KeyEvent.KEYCODE_INSERT -> TerminalKey.Insert
        KeyEvent.KEYCODE_SHIFT_LEFT -> TerminalKey.ShiftLeft
        KeyEvent.KEYCODE_SHIFT_RIGHT -> TerminalKey.ShiftRight
        KeyEvent.KEYCODE_CTRL_LEFT -> TerminalKey.ControlLeft
        KeyEvent.KEYCODE_CTRL_RIGHT -> TerminalKey.ControlRight
        KeyEvent.KEYCODE_ALT_LEFT -> TerminalKey.AltLeft
        KeyEvent.KEYCODE_ALT_RIGHT -> TerminalKey.AltRight
        KeyEvent.KEYCODE_CAPS_LOCK -> TerminalKey.CapsLock
        KeyEvent.KEYCODE_NUM_LOCK -> TerminalKey.NumLock
        KeyEvent.KEYCODE_SCROLL_LOCK -> TerminalKey.ScrollLock
        KeyEvent.KEYCODE_BREAK -> TerminalKey.Pause
        KeyEvent.KEYCODE_SYSRQ -> TerminalKey.PrintScreen
        KeyEvent.KEYCODE_MENU -> TerminalKey.Menu
        KeyEvent.KEYCODE_MINUS -> TerminalKey.Minus
        KeyEvent.KEYCODE_EQUALS -> TerminalKey.Equal
        KeyEvent.KEYCODE_LEFT_BRACKET -> TerminalKey.BracketLeft
        KeyEvent.KEYCODE_RIGHT_BRACKET -> TerminalKey.BracketRight
        KeyEvent.KEYCODE_SEMICOLON -> TerminalKey.Semicolon
        KeyEvent.KEYCODE_APOSTROPHE -> TerminalKey.Quote
        KeyEvent.KEYCODE_GRAVE -> TerminalKey.Backquote
        KeyEvent.KEYCODE_BACKSLASH -> TerminalKey.Backslash
        KeyEvent.KEYCODE_COMMA -> TerminalKey.Comma
        KeyEvent.KEYCODE_PERIOD -> TerminalKey.Period
        KeyEvent.KEYCODE_SLASH -> TerminalKey.Slash
        KeyEvent.KEYCODE_NUMPAD_MULTIPLY -> TerminalKey.NumpadMultiply
        KeyEvent.KEYCODE_NUMPAD_ADD -> TerminalKey.NumpadAdd
        KeyEvent.KEYCODE_NUMPAD_SUBTRACT -> TerminalKey.NumpadSubtract
        KeyEvent.KEYCODE_NUMPAD_DOT -> TerminalKey.NumpadDecimal
        KeyEvent.KEYCODE_NUMPAD_DIVIDE -> TerminalKey.NumpadDivide
        else -> null
    }

    fun modifiers(event: KeyEvent): KeyModifiers = KeyModifiers(
        shift = event.isShiftPressed, ctrl = event.isCtrlPressed, alt = event.isAltPressed,
        leftCtrl = event.metaState and KeyEvent.META_CTRL_LEFT_ON != 0,
        rightCtrl = event.metaState and KeyEvent.META_CTRL_RIGHT_ON != 0,
        leftAlt = event.metaState and KeyEvent.META_ALT_LEFT_ON != 0,
        rightAlt = event.metaState and KeyEvent.META_ALT_RIGHT_ON != 0,
        capsLock = event.isCapsLockOn, numLock = event.isNumLockOn, scrollLock = event.isScrollLockOn,
    )

    fun id(event: KeyEvent): Long = (event.deviceId.toLong() shl 32) or (event.keyCode.toLong() and 0xFFFFFFFFL)
}
