package ai.eclosion.octoterm.android.term

import android.content.Context
import android.os.Build
import android.text.InputType
import android.util.AttributeSet
import android.view.KeyEvent
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
        if (event.action == MotionEvent.ACTION_UP) {
            performClick()
            showKeyboard()
        }
        return true
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
                if (!text.isNullOrEmpty()) onInput(emulator.encodePaste(text.toString()))
                finishComposingText()
                return true
            }

            override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
                repeat(beforeLength.coerceAtLeast(0)) { onInput(byteArrayOf(0x7f)) }
                return true
            }

            override fun sendKeyEvent(event: KeyEvent): Boolean {
                return this@TerminalView.dispatchKeyEvent(event)
            }
        }
    }

    override fun onCheckIsTextEditor(): Boolean = true

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        if (event.action != KeyEvent.ACTION_DOWN) return super.onKeyDown(keyCode, event)
        when (keyCode) {
            KeyEvent.KEYCODE_DEL -> {
                onInput(byteArrayOf(0x7f))
                return true
            }
            KeyEvent.KEYCODE_ENTER -> {
                onInput(byteArrayOf('\r'.code.toByte()))
                return true
            }
            KeyEvent.KEYCODE_TAB -> {
                onInput(byteArrayOf('\t'.code.toByte()))
                return true
            }
            KeyEvent.KEYCODE_ESCAPE -> {
                onInput(byteArrayOf(0x1b))
                return true
            }
            KeyEvent.KEYCODE_DPAD_UP -> {
                onInput(emulator.encodeArrow(0, -1))
                return true
            }
            KeyEvent.KEYCODE_DPAD_DOWN -> {
                onInput(emulator.encodeArrow(0, 1))
                return true
            }
            KeyEvent.KEYCODE_DPAD_RIGHT -> {
                onInput(emulator.encodeArrow(1, 0))
                return true
            }
            KeyEvent.KEYCODE_DPAD_LEFT -> {
                onInput(emulator.encodeArrow(-1, 0))
                return true
            }
        }
        val unicode = event.unicodeChar
        if (unicode != 0) {
            onInput(String(Character.toChars(unicode)).toByteArray(Charsets.UTF_8))
            return true
        }
        return super.onKeyDown(keyCode, event)
    }
}
