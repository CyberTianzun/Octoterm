package ai.eclosion.octoterm.android.term

/** Logical keys retain their identity until the server-specific encoding step. */
enum class TerminalKey(val vk: Int, val scan: Int, val enhanced: Boolean = false, val character: Int = 0) {
    Escape(27, 1, character = 27), Backspace(8, 14, character = 8), Tab(9, 15, character = 9),
    Enter(13, 28, character = 13), Space(32, 57, character = 32),
    Up(38, 72, true), Down(40, 80, true), Left(37, 75, true), Right(39, 77, true),
    Home(36, 71, true), End(35, 79, true), PageUp(33, 73, true), PageDown(34, 81, true),
    Insert(45, 82, true), Delete(46, 83, true),
    ShiftLeft(16, 42), ShiftRight(16, 54), ControlLeft(17, 29), ControlRight(17, 29, true),
    AltLeft(18, 56), AltRight(18, 56, true), CapsLock(20, 58), NumLock(144, 69, true),
    ScrollLock(145, 70), Pause(19, 69), PrintScreen(44, 55, true), Menu(93, 93, true),
    A(65, 30, character = 97), B(66, 48, character = 98), C(67, 46, character = 99),
    D(68, 32, character = 100), E(69, 18, character = 101), F(70, 33, character = 102),
    G(71, 34, character = 103), H(72, 35, character = 104), I(73, 23, character = 105),
    J(74, 36, character = 106), K(75, 37, character = 107), L(76, 38, character = 108),
    M(77, 50, character = 109), N(78, 49, character = 110), O(79, 24, character = 111),
    P(80, 25, character = 112), Q(81, 16, character = 113), R(82, 19, character = 114),
    S(83, 31, character = 115), T(84, 20, character = 116), U(85, 22, character = 117),
    V(86, 47, character = 118), W(87, 17, character = 119), X(88, 45, character = 120),
    Y(89, 21, character = 121), Z(90, 44, character = 122),
    Digit0(48, 11, character = 48), Digit1(49, 2, character = 49), Digit2(50, 3, character = 50),
    Digit3(51, 4, character = 51), Digit4(52, 5, character = 52), Digit5(53, 6, character = 53),
    Digit6(54, 7, character = 54), Digit7(55, 8, character = 55), Digit8(56, 9, character = 56),
    Digit9(57, 10, character = 57),
    Minus(189, 12, character = 45), Equal(187, 13, character = 61),
    BracketLeft(219, 26, character = 91), BracketRight(221, 27, character = 93),
    Semicolon(186, 39, character = 59), Quote(222, 40, character = 39),
    Backquote(192, 41, character = 96), Backslash(220, 43, character = 92),
    Comma(188, 51, character = 44), Period(190, 52, character = 46), Slash(191, 53, character = 47),
    F1(112, 59), F2(113, 60), F3(114, 61), F4(115, 62), F5(116, 63), F6(117, 64),
    F7(118, 65), F8(119, 66), F9(120, 67), F10(121, 68), F11(122, 87), F12(123, 88),
    Numpad0(96, 82, character = 48), Numpad1(97, 79, character = 49), Numpad2(98, 80, character = 50),
    Numpad3(99, 81, character = 51), Numpad4(100, 75, character = 52), Numpad5(101, 76, character = 53),
    Numpad6(102, 77, character = 54), Numpad7(103, 71, character = 55), Numpad8(104, 72, character = 56),
    Numpad9(105, 73, character = 57), NumpadEnter(13, 28, true, 13),
    NumpadMultiply(106, 55, character = 42), NumpadAdd(107, 78, character = 43),
    NumpadSubtract(109, 74, character = 45), NumpadDecimal(110, 83, character = 46),
    NumpadDivide(111, 53, true, 47);

    companion object {
        fun fromAscii(char: Char): TerminalKey? = when (char) {
            in 'a'..'z', in 'A'..'Z' -> entries.first { it.name == char.uppercaseChar().toString() }
            in '0'..'9' -> entries.first { it.name == "Digit$char" }
            '\r', '\n' -> Enter
            '\t' -> Tab
            '\u001b' -> Escape
            ' ' -> Space
            else -> entries.firstOrNull { it.character == char.code && it.character != 0 }
                ?: when (char) {
                    '!' -> Digit1; '@' -> Digit2; '#' -> Digit3; '$' -> Digit4; '%' -> Digit5
                    '^' -> Digit6; '&' -> Digit7; '*' -> Digit8; '(' -> Digit9; ')' -> Digit0
                    '_' -> Minus; '+' -> Equal; '{' -> BracketLeft; '}' -> BracketRight
                    ':' -> Semicolon; '"' -> Quote; '~' -> Backquote; '|' -> Backslash
                    '<' -> Comma; '>' -> Period; '?' -> Slash
                    else -> null
                }
        }
    }
}

data class KeyModifiers(
    val shift: Boolean = false,
    val ctrl: Boolean = false,
    val alt: Boolean = false,
    val leftCtrl: Boolean = false,
    val rightCtrl: Boolean = false,
    val leftAlt: Boolean = false,
    val rightAlt: Boolean = false,
    val capsLock: Boolean = false,
    val numLock: Boolean = false,
    val scrollLock: Boolean = false,
) {
    val active: Boolean get() = shift || ctrl || alt

    operator fun plus(other: KeyModifiers): KeyModifiers = copy(
        shift = shift || other.shift, ctrl = ctrl || other.ctrl, alt = alt || other.alt,
        leftCtrl = leftCtrl || other.leftCtrl, rightCtrl = rightCtrl || other.rightCtrl,
        leftAlt = leftAlt || other.leftAlt, rightAlt = rightAlt || other.rightAlt,
        capsLock = capsLock || other.capsLock, numLock = numLock || other.numLock,
        scrollLock = scrollLock || other.scrollLock,
    )

    fun windowsState(enhanced: Boolean): Int {
        var state = if (enhanced) 0x100 else 0
        if (shift) state = state or 0x10
        if (ctrl) {
            if (rightCtrl) state = state or 0x4
            if (leftCtrl || !rightCtrl) state = state or 0x8
        }
        if (alt) {
            if (rightAlt) state = state or 0x1
            if (leftAlt || !rightAlt) state = state or 0x2
        }
        if (capsLock) state = state or 0x80
        if (numLock) state = state or 0x20
        if (scrollLock) state = state or 0x40
        return state
    }
}

data class KeyStroke(
    val key: TerminalKey,
    val codePoint: Int = key.character,
    val modifiers: KeyModifiers = KeyModifiers(),
)

/** One instance per session. Physical key-up uses the identity saved at key-down. */
class TerminalKeyboard(val serverOs: String) {
    private data class Pressed(val stroke: KeyStroke, val win32: Boolean, val character: Int)
    private val pressed = linkedMapOf<Long, Pressed>()

    fun hasPressed(id: Long): Boolean = pressed.containsKey(id)

    fun keyDown(id: Long, stroke: KeyStroke, applicationCursor: Boolean = false): ByteArray {
        val win32 = serverOs == "windows" && stroke.codePoint in 0..127
        pressed[id] = Pressed(stroke, win32, character(stroke))
        return if (win32) packet(stroke, true) else legacy(stroke, applicationCursor)
    }

    fun keyUp(id: Long, modifiers: KeyModifiers): ByteArray? {
        val previous = pressed.remove(id) ?: return null
        return if (previous.win32) {
            packet(previous.stroke.copy(modifiers = modifiers), false, previous.character)
        } else byteArrayOf()
    }

    fun tap(stroke: KeyStroke, applicationCursor: Boolean = false): ByteArray =
        if (serverOs == "windows" && stroke.codePoint in 0..127) {
            packet(stroke, true) + packet(stroke, false)
        } else legacy(stroke, applicationCursor)

    /** IME commits are text, never bracketed paste. Explicit modifiers apply to one ASCII key. */
    fun committedText(text: String, modifiers: KeyModifiers = KeyModifiers()): ByteArray {
        if (text.length == 1 && text[0].code < 128) {
            val key = TerminalKey.fromAscii(text[0])
            if (key != null && (modifiers.active || text == "\n" || text == "\r" || text == "\t")) {
                val cp = if (modifiers.shift && text[0] in 'a'..'z') text[0].uppercaseChar().code else text[0].code
                return tap(KeyStroke(key, cp, modifiers))
            }
        }
        return text.toByteArray(Charsets.UTF_8)
    }

    fun releaseAll(): ByteArray {
        val packets = pressed.values.filter { it.win32 }.map {
            packet(it.stroke.copy(modifiers = KeyModifiers()), false, it.character).toString(Charsets.US_ASCII)
        }.joinToString("")
        reset()
        return packets.toByteArray(Charsets.US_ASCII)
    }

    fun reset() = pressed.clear()

    private fun packet(stroke: KeyStroke, down: Boolean, savedCharacter: Int? = null): ByteArray {
        val key = stroke.key
        val cp = stroke.codePoint
        var vk = if (key in TerminalKey.A..TerminalKey.Z && cp.toChar().isAsciiLetter()) {
            cp.toChar().uppercaseChar().code
        } else key.vk
        if (key in TerminalKey.Numpad0..TerminalKey.Numpad9 && cp == 0) {
            vk = listOf(45, 35, 40, 34, 37, 12, 39, 36, 38, 33)[key.ordinal - TerminalKey.Numpad0.ordinal]
        }
        if (key == TerminalKey.NumpadDecimal && cp == 0) vk = 46
        val char = savedCharacter ?: character(stroke)
        val state = stroke.modifiers.windowsState(key.enhanced)
        return "\u001b[$vk;${key.scan};$char;${if (down) 1 else 0};$state;1_".toByteArray(Charsets.US_ASCII)
    }

    private fun character(stroke: KeyStroke): Int {
        val key = stroke.key
        val ctrl = stroke.modifiers.ctrl
        return when (key) {
            TerminalKey.Enter, TerminalKey.NumpadEnter -> if (ctrl) 10 else 13
            TerminalKey.Tab -> 9
            TerminalKey.Backspace -> if (ctrl) 127 else 8
            TerminalKey.Escape -> 27
            else -> controlCharacter(stroke.codePoint, ctrl)
        }
    }

    private fun legacy(stroke: KeyStroke, applicationCursor: Boolean): ByteArray {
        val key = stroke.key
        val m = stroke.modifiers
        val modifier = 1 + (if (m.shift) 1 else 0) + (if (m.alt) 2 else 0) + (if (m.ctrl) 4 else 0)
        val letter = when (key) {
            TerminalKey.Up -> 'A'; TerminalKey.Down -> 'B'; TerminalKey.Right -> 'C'; TerminalKey.Left -> 'D'
            TerminalKey.Home -> 'H'; TerminalKey.End -> 'F'
            TerminalKey.F1 -> 'P'; TerminalKey.F2 -> 'Q'; TerminalKey.F3 -> 'R'; TerminalKey.F4 -> 'S'
            else -> null
        }
        if (letter != null) {
            val seq = when {
                modifier != 1 -> "\u001b[1;$modifier$letter"
                key in TerminalKey.F1..TerminalKey.F4 || applicationCursor -> "\u001bO$letter"
                else -> "\u001b[$letter"
            }
            return seq.toByteArray(Charsets.US_ASCII)
        }
        val number = when (key) {
            TerminalKey.Insert -> 2; TerminalKey.Delete -> 3; TerminalKey.PageUp -> 5; TerminalKey.PageDown -> 6
            TerminalKey.F5 -> 15; TerminalKey.F6 -> 17; TerminalKey.F7 -> 18; TerminalKey.F8 -> 19
            TerminalKey.F9 -> 20; TerminalKey.F10 -> 21; TerminalKey.F11 -> 23; TerminalKey.F12 -> 24
            else -> null
        }
        if (number != null) {
            return "\u001b[$number${if (modifier == 1) "" else ";$modifier"}~".toByteArray(Charsets.US_ASCII)
        }
        if (key == TerminalKey.Tab && m.shift) return "\u001b[Z".toByteArray(Charsets.US_ASCII)
        if (key.character == 0 && stroke.codePoint == 0) return byteArrayOf()
        val cp = if (key == TerminalKey.Backspace) { if (m.ctrl) 8 else 127 } else character(stroke)
        val text = String(Character.toChars(cp))
        return ((if (m.alt) "\u001b" else "") + text).toByteArray(Charsets.UTF_8)
    }

    private fun controlCharacter(cp: Int, ctrl: Boolean): Int = when {
        !ctrl -> cp
        cp == 32 || cp == 64 -> 0
        cp == 47 || cp == 95 -> 31
        cp == 63 -> 127
        cp in 50..56 -> listOf(0, 27, 28, 29, 30, 31, 127)[cp - 50]
        cp in 64..95 || cp in 97..122 -> cp and 31
        else -> cp
    }
}

private fun Char.isAsciiLetter(): Boolean = this in 'a'..'z' || this in 'A'..'Z'
