package ai.eclosion.octoterm.android.term

/**
 * 首帧/resync 期间的刷屏门闩。
 *
 * 服务端顺序是 resized → resync-begin → 重绘字节 → resync-end。
 * begin 会把模拟器清空；若这时把 generation 推给 Compose，空屏会盖住
 * 刚画好的一帧。二次 layout 上报的 resize 再来一次 resized，又会用
 * 本地裁切后的网格覆盖正确重绘。
 *
 * 持有期间只改模拟器、不提交画面；客户端尺寸要等第一次 resync-end
 * 之后再报。
 */
internal class TermPaintGate {
    var holding: Boolean = false
        private set
    var acceptClientResize: Boolean = false
        private set

    fun onSessionOpen() {
        holding = true
        acceptClientResize = false
    }

    fun onAttached(replay: Boolean) {
        if (replay) {
            holding = false
            acceptClientResize = true
        } else {
            holding = true
            acceptClientResize = false
        }
    }

    fun onResyncBegin() {
        holding = true
    }

    fun onResyncEnd() {
        holding = false
        acceptClientResize = true
    }

    fun onDetached() {
        holding = false
        acceptClientResize = false
    }

    fun shouldPublishWrite(): Boolean = !holding

    fun shouldSendResize(): Boolean = acceptClientResize && !holding
}
