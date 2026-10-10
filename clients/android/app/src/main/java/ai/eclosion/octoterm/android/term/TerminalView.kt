package ai.eclosion.octoterm.android.term

import android.content.Context
import android.graphics.Rect
import android.os.Build
import android.text.InputType
import android.util.AttributeSet
import android.view.KeyEvent
import android.view.KeyCharacterMap
import android.view.MotionEvent
import android.view.View
import android.view.WindowInsets
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager

class TerminalView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : View(context, attrs) {
    var emulator: VtEmulator = VtEmulator()

    var onInput: (ByteArray) -> Unit = {}
    var keyboard: TerminalKeyboard = TerminalKeyboard("")
    var softModifiers: KeyModifiers = KeyModifiers()
    var onModifiersConsumed: () -> Unit = {}
    var inputEnabled: Boolean = true
    private var composing = false
    private var deadAccent = 0
    private val textKeys = mutableSetOf<Long>()

    init {
        isFocusable = true
        isFocusableInTouchMode = true
        isClickable = true
        keepScreenOn = true
        setBackgroundColor(android.graphics.Color.TRANSPARENT)
        setWillNotDraw(true)
    }

    fun showKeyboard() {
        isFocusable = true
        isFocusableInTouchMode = true
        requestFocus()
        val imm = context.getSystemService(InputMethodManager::class.java) ?: return
        imm.viewClicked(this)
        imm.restartInput(this)
        post {
            requestFocus()
            imm.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT)
            if (Build.VERSION.SDK_INT >= 30) {
                windowInsetsController?.show(WindowInsets.Type.ime())
            }
        }
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        // 滚动、选区和鼠标上报在 Compose 层处理。这个 View 只负责输入法。
        return false
    }

    override fun performClick(): Boolean {
        super.performClick()
        return true
    }

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        // TYPE_NULL 在 Gboard / 三星键盘上经常干脆不弹盘。VISIBLE_PASSWORD
        // 是终端里常用的「能弹盘、少联想」折中（Termux 同款）。
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or
            InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD or
            InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN or
            EditorInfo.IME_FLAG_NO_EXTRACT_UI or
            EditorInfo.IME_ACTION_NONE
        return object : BaseInputConnection(this, true) {
            override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
                if (inputEnabled && !text.isNullOrEmpty()) {
                    val value = text.toString()
                    val asciiKey = value.length == 1 && value[0].code in 32..126
                    val modifiers = if (composing && !asciiKey) KeyModifiers() else softModifiers
                    emit(keyboard.committedText(value, modifiers))
                    if ((!composing || asciiKey) && value.length == 1 && value[0].code < 128 &&
                        TerminalKey.fromAscii(value[0]) != null
                    ) consumeModifiers()
                }
                finishComposingText()
                editable?.clear()
                return true
            }

            override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
                composing = !text.isNullOrEmpty()
                return super.setComposingText(text, newCursorPosition)
            }

            override fun finishComposingText(): Boolean {
                composing = false
                return super.finishComposingText()
            }

            override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
                if (composing) return super.deleteSurroundingText(beforeLength, afterLength)
                if (inputEnabled) {
                    repeat(beforeLength.coerceIn(0, 200_000)) {
                        emit(keyboard.tap(KeyStroke(TerminalKey.Backspace, modifiers = softModifiers)))
                    }
                    repeat(afterLength.coerceIn(0, 200_000)) {
                        emit(keyboard.tap(KeyStroke(TerminalKey.Delete, modifiers = softModifiers)))
                    }
                    consumeModifiers()
                }
                return true
            }

            override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean =
                if (composing) super.deleteSurroundingTextInCodePoints(beforeLength, afterLength)
                else deleteSurroundingText(beforeLength, afterLength)

            override fun sendKeyEvent(event: KeyEvent): Boolean {
                return this@TerminalView.dispatchKeyEvent(event)
            }
        }
    }

    override fun onCheckIsTextEditor(): Boolean = true

    // Capture hardware shortcuts before an IME turns them into an unmodified text commit.
    override fun onKeyPreIme(keyCode: Int, event: KeyEvent): Boolean {
        val key = AndroidKeyMapper.key(keyCode)
        val shortcut = event.isCtrlPressed || event.isAltPressed ||
            (event.isShiftPressed && key?.character in listOf(0, 8, 9, 13, 27))
        if (!composing && !event.isMetaPressed && key != null &&
            event.flags and KeyEvent.FLAG_SOFT_KEYBOARD == 0 &&
            (shortcut || keyboard.hasPressed(AndroidKeyMapper.id(event)))
        ) {
            return when (event.action) {
                KeyEvent.ACTION_DOWN -> onKeyDown(keyCode, event)
                KeyEvent.ACTION_UP -> onKeyUp(keyCode, event)
                else -> super.onKeyPreIme(keyCode, event)
            }
        }
        return super.onKeyPreIme(keyCode, event)
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        if (event.action != KeyEvent.ACTION_DOWN) return super.onKeyDown(keyCode, event)
        if (event.isMetaPressed) return super.onKeyDown(keyCode, event)
        var key = AndroidKeyMapper.key(keyCode)
        val id = AndroidKeyMapper.id(event)
        if (!inputEnabled) return key != null || super.onKeyDown(keyCode, event)
        if (composing) return super.onKeyDown(keyCode, event)
        val altGr = event.isCtrlPressed && event.metaState and KeyEvent.META_ALT_RIGHT_ON != 0
        val meta = if (altGr) event.metaState else event.metaState and
            (KeyEvent.META_CTRL_MASK or KeyEvent.META_ALT_MASK).inv()
        var unicode = event.getUnicodeChar(meta)
        if (unicode and KeyCharacterMap.COMBINING_ACCENT != 0) {
            deadAccent = unicode and KeyCharacterMap.COMBINING_ACCENT_MASK
            textKeys.add(id)
            return true
        }
        if (deadAccent != 0 && unicode != 0) {
            val composed = KeyEvent.getDeadChar(deadAccent, unicode)
            val text = if (composed != 0) String(Character.toChars(composed)) else
                String(Character.toChars(deadAccent)) + String(Character.toChars(unicode))
            deadAccent = 0
            textKeys.add(id)
            emit(text.toByteArray(Charsets.UTF_8))
            return true
        }
        if (altGr && unicode != 0) {
            textKeys.add(id)
            emit(String(Character.toChars(unicode)).toByteArray(Charsets.UTF_8))
            return true
        }
        if (key == null) {
            key = if (unicode in 1..127) TerminalKey.fromAscii(unicode.toChar()) else null
            if (key == null) {
                if (unicode == 0) return super.onKeyDown(keyCode, event)
                textKeys.add(id)
                emit(String(Character.toChars(unicode)).toByteArray(Charsets.UTF_8))
                return true
            }
        }
        if (unicode == 0 && key.character != 0 && key !in TerminalKey.Numpad0..TerminalKey.Numpad9 &&
            key != TerminalKey.NumpadDecimal
        ) unicode = key.character
        val modifiers = AndroidKeyMapper.modifiers(event) + softModifiers
        emit(keyboard.keyDown(id, KeyStroke(key, unicode, modifiers), emulator.applicationCursor))
        if (key !in TerminalKey.ShiftLeft..TerminalKey.ScrollLock) consumeModifiers()
        return true
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean {
        val id = AndroidKeyMapper.id(event)
        val textKey = textKeys.remove(id)
        val bytes = keyboard.keyUp(id, AndroidKeyMapper.modifiers(event))
            ?: return if (textKey) true else super.onKeyUp(keyCode, event)
        if (inputEnabled) emit(bytes)
        return true
    }

    fun releaseKeys() {
        if (inputEnabled) emit(keyboard.releaseAll()) else keyboard.reset()
        textKeys.clear()
        deadAccent = 0
        consumeModifiers()
    }

    override fun onFocusChanged(gainFocus: Boolean, direction: Int, previouslyFocusedRect: Rect?) {
        if (!gainFocus) releaseKeys()
        super.onFocusChanged(gainFocus, direction, previouslyFocusedRect)
    }

    override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
        if (!hasWindowFocus) releaseKeys()
        super.onWindowFocusChanged(hasWindowFocus)
    }

    override fun onDetachedFromWindow() {
        releaseKeys()
        super.onDetachedFromWindow()
    }

    private fun emit(bytes: ByteArray) { if (bytes.isNotEmpty()) onInput(bytes) }

    private fun consumeModifiers() {
        softModifiers = KeyModifiers()
        onModifiersConsumed()
    }
}
