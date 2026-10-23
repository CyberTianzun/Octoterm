package ai.eclosion.octoterm.android.appearance

import android.content.Context
import android.graphics.Typeface

/**
 * 终端字体按需加载器：确保内存中同一时刻最多只保留一个内置字体实例。
 */
object TermFontLoader {
    private var activeKind: TermFontKind? = null
    private var activeTypeface: Typeface? = null

    @Synchronized
    fun load(context: Context, kind: TermFontKind): Typeface {
        if (activeKind == kind && activeTypeface != null) {
            return activeTypeface!!
        }
        // 立即释放前一个自定义字体引用，保证内存里同一时刻只保留一个字体实例
        activeTypeface = null
        activeKind = null

        val assetPath = kind.assetPath
        val typeface = if (assetPath == null) {
            Typeface.MONOSPACE
        } else {
            runCatching {
                Typeface.createFromAsset(context.applicationContext.assets, assetPath)
            }.getOrDefault(Typeface.MONOSPACE)
        }
        activeKind = kind
        activeTypeface = typeface
        return typeface
    }

    @Synchronized
    internal fun activeKindForTest(): TermFontKind? = activeKind
}
