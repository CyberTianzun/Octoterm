/**
 * 内置终端字体目录与按需单字体内存加载器。
 *
 * 约束:
 * 1. 多端一致:Web 与 Android 统一内置 JetBrains Mono、Google Sans Code、Zed Mono 三款开源等宽字体,
 *    外加系统默认等宽字体栈(Windows 下含 Consolas)。
 * 2. **内存中最多只加载一个内置字体**:切换字体时,先从 `document.fonts` 移除并释放上一个内置 `FontFace`,
 *    再按需加载当前选中的 `.woff2`,绝不同时驻留多份字体数据。
 */

export const SYSTEM_MONO_STACK =
  'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace';

export interface BuiltinFontPreset {
  id: "jetbrains-mono" | "google-sans-code" | "zed-mono" | "monospace";
  label: string;
  family: string;
  stack: string;
  /** 内置字体文件路径;系统等宽栈不走网络/内存字体加载。 */
  url?: string;
}

export const BUILTIN_FONTS: readonly BuiltinFontPreset[] = [
  {
    id: "jetbrains-mono",
    label: "JetBrains Mono",
    family: "JetBrains Mono",
    stack: `"JetBrains Mono", ${SYSTEM_MONO_STACK}`,
    url: "/fonts/JetBrainsMono-Regular.woff2",
  },
  {
    id: "google-sans-code",
    label: "Google Sans Code",
    family: "Google Sans Code",
    stack: `"Google Sans Code", ${SYSTEM_MONO_STACK}`,
    url: "/fonts/GoogleSansCode-Regular.woff2",
  },
  {
    id: "zed-mono",
    label: "Zed Mono",
    family: "Zed Mono",
    stack: `"Zed Mono", ${SYSTEM_MONO_STACK}`,
    url: "/fonts/ZedMono-Regular.woff2",
  },
  {
    id: "monospace",
    label: "Monospace (Consolas / System)",
    family: "monospace",
    stack: SYSTEM_MONO_STACK,
  },
] as const;

/** 根据 CSS font-family 栈识别最优先匹配的内置字体预设。 */
export function matchBuiltinFont(stack: string): BuiltinFontPreset | undefined {
  const parts = stack
    .split(",")
    .map((s) => s.trim().replace(/^["']|["']$/g, "").toLowerCase())
    .filter(Boolean);
  for (const part of parts) {
    for (const preset of BUILTIN_FONTS) {
      if (preset.url && part === preset.family.toLowerCase()) {
        return preset;
      }
    }
    if (part.includes("jetbrains")) return BUILTIN_FONTS[0];
    if (part.includes("google sans code") || part.includes("googlesans")) return BUILTIN_FONTS[1];
    if (part.includes("zed mono") || part === "zed") return BUILTIN_FONTS[2];
  }
  return BUILTIN_FONTS.find((p) => p.id === "monospace");
}

let activeFontFace: FontFace | null = null;
let activeFontId: string | null = null;
let loadSeq = 0;

function unloadActiveFont(): void {
  if (activeFontFace && typeof document !== "undefined" && document.fonts) {
    try {
      (document.fonts as unknown as { delete(f: FontFace): boolean }).delete(activeFontFace);
    } catch {
      /* 忽略旧浏览器兼容异常 */
    }
  }
  activeFontFace = null;
  activeFontId = null;
}

/**
 * 确保当前配置选中的内置字体已加载到内存中,并卸载之前加载过的其它内置字体。
 * 返回 `true` 表示本次新加载了一个内置字体(调用方应刷新 xterm 纹理集与尺寸测量)。
 */
export async function ensureFontLoaded(stack: string): Promise<boolean> {
  if (typeof FontFace !== "function" || typeof document === "undefined" || !document.fonts) {
    return false;
  }
  const preset = matchBuiltinFont(stack);
  if (!preset || !preset.url) {
    const hadActive = activeFontFace !== null;
    unloadActiveFont();
    return hadActive;
  }
  if (activeFontId === preset.id && activeFontFace !== null) {
    return false;
  }

  // 立即释放上一个内置字体,保证内存里同一时刻最多只有一份自定义字体
  unloadActiveFont();

  const seq = ++loadSeq;
  try {
    const face = new FontFace(preset.family, `url(${preset.url})`, {
      style: "normal",
      weight: "400",
      display: "swap",
    });
    const loaded = await face.load();
    if (seq !== loadSeq) {
      return false;
    }
    (document.fonts as unknown as { add(f: FontFace): void }).add(loaded);
    activeFontFace = loaded;
    activeFontId = preset.id;
    return true;
  } catch (err) {
    console.warn(`octoterm: 内置字体 ${preset.family} 加载失败,回退系统等宽字体`, err);
    return false;
  }
}

/** 供测试或调试读取当前驻留内存的内置字体 ID。 */
export function getActiveLoadedFontId(): string | null {
  return activeFontId;
}
