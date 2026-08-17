/**
 * 主题目录:一小撮内置主题编进 bundle,数百个的全量表按需拉。
 * (具体数量由 scripts/gen-themes.mjs 的 BUILTIN 决定,这里不写死。)
 *
 * 首屏只需要「当前选中的那一个」,而当前选中的主题的颜色是内联在配置里的(见
 * config.ts 对 theme.colors 的说明),所以全量表只在用户真的打开主题选择器时
 * 才值得付那几十 KB。拉失败就退回内置的那几个 —— 离线时选择面变窄,但不报错、
 * 不阻塞,已经在用的主题一点不受影响。
 */
import type { ITheme } from "@xterm/xterm";
import { BUILTIN_THEMES, DEFAULT_THEMES } from "./themes/builtin";

export { BUILTIN_THEMES, DEFAULT_THEMES };

const CATALOG_URL = "./themes.json";

let catalog: Record<string, ITheme> | null = null;
let inflight: Promise<Record<string, ITheme>> | null = null;

/** 同步查表。全量表还没拉到时只查得到内置那几个 —— 够用了:调用点(配置
 *  反序列化)只在配置没内联颜色时才需要它。 */
export function resolveTheme(name: string): ITheme | undefined {
  return catalog?.[name] ?? BUILTIN_THEMES[name];
}

/** 当前能查到的全部主题。拉过就是全量表,没拉过就是内置那几个。 */
export function knownThemes(): Record<string, ITheme> {
  return catalog ?? BUILTIN_THEMES;
}

export function catalogLoaded(): boolean {
  return catalog !== null;
}

/** 拉全量表。并发调用共享同一个请求;失败时退回内置表且不缓存,下次还能重试。 */
export async function loadCatalog(): Promise<Record<string, ITheme>> {
  if (catalog) return catalog;
  if (inflight) return inflight;
  inflight = (async () => {
    try {
      const res = await fetch(CATALOG_URL);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = (await res.json()) as Record<string, ITheme>;
      // 内置那几个合并进来兜底:万一 themes.json 是旧版本、少了某个主题,
      // 当前配置引用的主题名仍然查得到。
      catalog = { ...BUILTIN_THEMES, ...data };
      return catalog;
    } catch (err) {
      console.warn("octoterm: 主题目录加载失败,仅使用内置主题", err);
      return BUILTIN_THEMES;
    } finally {
      inflight = null;
    }
  })();
  return inflight;
}
