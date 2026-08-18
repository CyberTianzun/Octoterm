/**
 * 「新建会话」菜单里的启动项。
 *
 * 列表由服务端的 provider 扫出来(内置默认 shell、用户 config.toml、iTerm2、
 * Windows Terminal),客户端只负责显示和把选中的那条原样发回去 —— 浏览器读不了
 * 本地配置文件,这件事只能在服务端做。
 *
 * 走 HTTP 而不是控制通道:这是一份与会话无关的清单,页面加载时就要用,那时
 * WebSocket 可能还没握手完。
 */
import { t } from "./i18n";

export interface Launcher {
  id: string;
  provider: string;
  name: string;
  /** 一行命令预览,给人看的 */
  detail: string;
  /** 直接可以 spawn 的 argv;空数组表示「让服务端用它的默认 shell」 */
  command: string[];
  cwd: string | null;
}

/**
 * 兜底项。列表拉不到(服务端旧版本、端点报错、断网)时菜单里至少有它 ——
 * 「新建会话」是这个工具最基本的动作,不能因为一份第三方配置读不了就用不了。
 * `command: []` 让 new-session 发 `command: null`,由服务端决定跑什么。
 *
 * 是函数而不是常量:文案跟着界面语言走,而语言可以在运行期改。菜单每次打开
 * 都重新拉一次列表,所以现取现算就够了。
 */
export function defaultLauncher(): Launcher {
  return {
    id: "fallback:default",
    provider: "builtin",
    name: t("launcher.defaultName"),
    detail: t("launcher.defaultDetail"),
    command: [],
    cwd: null,
  };
}

/** 品牌名(iTerm2 / Windows Terminal)不翻译:它们是产品名,翻了反而认不出。 */
export function providerLabel(provider: string): string {
  switch (provider) {
    case "builtin":
      return t("launcher.provider.builtin");
    case "config":
      return t("launcher.provider.config");
    case "iterm2":
      return "iTerm2";
    case "windows-terminal":
      return "Windows Terminal";
    default:
      return provider;
  }
}

/** 只保留结构完整的条目:服务端是可以被换掉的,别信它一定给对。 */
function sanitize(raw: unknown): Launcher[] {
  if (!Array.isArray(raw)) return [];
  const out: Launcher[] = [];
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const l = item as Record<string, unknown>;
    if (typeof l.id !== "string" || !l.id) continue;
    if (typeof l.name !== "string" || !l.name) continue;
    if (!Array.isArray(l.command) || !l.command.every((a) => typeof a === "string")) continue;
    if (l.command.length === 0) continue;
    out.push({
      id: l.id,
      provider: typeof l.provider === "string" ? l.provider : "unknown",
      name: l.name,
      detail: typeof l.detail === "string" ? l.detail : (l.command as string[]).join(" "),
      command: l.command as string[],
      cwd: typeof l.cwd === "string" && l.cwd ? l.cwd : null,
    });
  }
  return out;
}

/**
 * 拉一次启动项。**永不抛异常**:任何失败都退化成只有兜底项的列表,菜单照常能用。
 * 返回值保证非空。
 */
export async function fetchLaunchers(token: string): Promise<Launcher[]> {
  try {
    const res = await fetch("/api/launchers", {
      headers: { Authorization: `Bearer ${token}` },
    });
    if (!res.ok) {
      console.warn("octoterm: 启动项列表返回", res.status);
      return [defaultLauncher()];
    }
    const body = (await res.json()) as { launchers?: unknown };
    const list = sanitize(body?.launchers);
    return list.length > 0 ? list : [defaultLauncher()];
  } catch (err) {
    console.warn("octoterm: 启动项列表拉取失败", err);
    return [defaultLauncher()];
  }
}
