/**
 * 界面多语言。
 *
 * 三条约束(和 config.ts 同源):
 *
 * 1. **纯模块,不碰 DOM**。语言探测要读 navigator,那是副作用,单独放在
 *    `detectLocale()` 里由调用方在边界上调;`resolveLocale()` 本身是纯函数,
 *    能被 node:test 直接跑。config.ts 的 warning 也走这里,所以这个模块必须
 *    能在 node 里 import。
 * 2. **两份词条都编进 bundle**。总共百来条,几 KB,不值得为它多一次网络往返
 *    ——首屏就要用文案,异步加载只会换来一次闪烁。
 * 3. **zh-CN 是词条的真相来源**:`MsgKey` 由它推导,别的语言少一条键就是类型
 *    错误(`tsc --noEmit` 拦得住),不会静默漏翻。
 */

export type Locale = "zh-CN" | "en";
/** 用户的语言偏好。"auto" = 跟随浏览器。 */
export type LocalePref = "auto" | Locale;

export const LOCALES = ["en", "zh-CN"] as const;
/** 语言自己的名字:选择器里永远用母语显示,英文界面下也要让中文用户认得出。 */
export const LOCALE_NAMES: Record<Locale, string> = {
  en: "English",
  "zh-CN": "简体中文",
};
/** `<html lang>` 和 `toLocaleString()` 用的 BCP 47 标签。 */
export const LOCALE_TAGS: Record<Locale, string> = {
  en: "en",
  "zh-CN": "zh-CN",
};

const zh = {
  /* ---------- 主界面 ---------- */
  "app.settings": "设置",
  "app.newSession": "新建会话",
  "app.sessionList": "会话列表",
  "app.kernel": "本地会话内核",
  "app.empty": "还没有会话。点右上角 + 新建一个,或者在左侧选一个。",
  "app.backToList": "回到会话列表",
  "session.listTitle": "会话",
  "session.open": "打开",
  "app.tokenPrompt": "octoterm token:",

  /* ---------- 连接状态 ---------- */
  "conn.connected": "已连接",
  "conn.reconnecting": "重连中",
  "conn.disconnected": "已断开",
  "conn.banner.reconnecting": "正在重连…",
  "conn.banner.fatal": "连接失败:{message} — 刷新页面重新输入 token",

  /* ---------- 会话 ---------- */
  "session.rename": "改名",
  "session.kill": "结束会话",
  "session.renamePrompt": "新名字:",

  /* ---------- agent 集成 ---------- */
  "agent.state.idle": "空闲",
  "agent.state.thinking": "思考中",
  "agent.state.working": "执行中",
  "agent.state.waiting": "等你回答",
  "agent.state.waitingTerminal": "在终端里等你",
  "agent.notice.permission": "在等授权",
  "agent.notice.idle": "闲着等你说话",
  "agent.notice.input": "需要你的输入",
  "agent.notice.choice": "在等你选择",
  "agent.denyReason": "拒绝的理由(可留空)",
  "agent.expiresIn": "还有 {n} 秒",
  "agent.expired": "已过期",
  "agent.unknownTool": "未说明的操作",
  "agent.reviewHint": "看清楚下面这条再决定 —— 允许之后它会立刻执行",
  "agent.state.done": "已结束",
  "agent.state.error": "出错",
  "agent.waitingTitle": "有 AI 在等你回答",
  "agent.allow": "允许",
  "agent.deny": "拒绝",
  "agent.openSession": "去这个会话",
  "agent.gone": "这个请求已经过期了",
  "agent.already": "别的设备已经答过了",
  "agent.failed": "回答没送到,请重试",
  "settings.tab.agents": "AI 集成",
  "agent.install": "安装",
  "agent.uninstall": "卸载",
  "agent.installed": "已安装",
  "agent.notInstalled": "未安装",
  "agent.stalePort": "已安装,但端口对不上(远程接管此刻不生效)",
  "agent.notFound": "本机没有找到可集成的 AI",
  "agent.disabledHint": "服务端未开启安装功能(config.toml 里的 agents.install_enabled)",
  "agent.installFailed": "操作失败",
  "agent.conflictNote": "检测到别的程序也在这个事件上挂了阻塞式 hook,只会安装状态上报部分",
  "agent.writeNote": "安装会修改这个 AI 自己的配置文件,改前会自动备份;卸载可完全还原",
  "agent.preview": "看看会改什么",
  "agent.previewHide": "收起",
  "agent.previewFailed": "读不到预演结果",
  "agent.previewAdd": "写入这些事件的 hook:",
  "agent.previewRemove": "移除这些事件的 hook:",
  "agent.previewTelemetryOnly": "只装状态上报,不装授权拦截(见上方冲突)",
  "agent.activation.codex-hooks-review": "装好之后还要在 Codex 里执行 /hooks 审核一遍才会生效 —— 这是 Codex 自己的安全闸,只能由你来点头",
  "session.empty": "还没有会话,点右上角 + 新建",

  /* ---------- 新建会话菜单 ---------- */
  "ns.filter": "筛选…",
  "ns.filterAria": "筛选启动项",
  "ns.loading": "载入中…",
  "ns.noMatch": "没有匹配的启动项",

  /* ---------- 启动项 ---------- */
  "launcher.defaultName": "默认 shell",
  "launcher.defaultDetail": "由服务端决定",
  "launcher.provider.builtin": "内置",
  "launcher.provider.config": "自定义 (config.toml)",

  /* ---------- 设置面板 ---------- */
  "settings.close": "关闭 (Esc)",
  "settings.tab.theme": "主题",
  "settings.tab.font": "字体",
  "settings.tab.terminal": "终端",
  "settings.tab.ui": "界面",
  "settings.tab.io": "导入 / 导出",

  "settings.theme.search": "搜索 {n} 个主题…",
  "settings.theme.searchLoading": "搜索主题(正在加载完整目录…)",
  "settings.theme.noMatch": "没有匹配的主题",
  "settings.theme.note":
    "主题数据来自 mbadolato/iTerm2-Color-Schemes(MIT)。想要目录里没有的配色,在「导入 / 导出」里直接改 theme.colors 即可。",

  "settings.font.family": "字体栈",
  "settings.font.familyHint": "CSS font-family,逗号分隔",
  "settings.font.available": "✓ 首选族「{name}」可用",
  "settings.font.missing": "⚠ 系统里找不到「{name}」,会回落到栈里的下一个",
  "settings.font.reset": "恢复默认字体栈",
  "settings.font.size": "字号",
  "settings.font.lineHeight": "行高",
  "settings.font.letterSpacing": "字距",
  "settings.font.weight": "常规字重",
  "settings.font.weightBold": "加粗字重",

  "settings.term.cursorStyle": "光标形状",
  "settings.term.cursorInactive": "失焦光标",
  "settings.term.cursorBlink": "光标闪烁",
  "settings.term.scrollback": "回滚行数",
  "settings.term.customGlyphs": "内置字形绘制",
  "settings.term.customGlyphsHint": "自己画 box-drawing / powerline,不依赖 Nerd Font",
  "settings.term.contrast": "最小对比度",
  "settings.term.contrastHint": "1 = 不干预;提高可读性但会改写主题色",
  "settings.term.boldBright": "加粗用亮色",

  "settings.ui.language": "语言",
  "settings.ui.languageAuto": "跟随浏览器",
  "settings.ui.languageHint": "立即生效,不需要刷新页面",
  "settings.ui.sidebarPreview": "侧边栏会话预览",
  "settings.ui.webgl": "WebGL 渲染器",
  "settings.ui.webglHint": "关闭则回落到 DOM 渲染器",

  "settings.io.note":
    "配置就是下面这段 JSON,可以直接改。theme.colors 是 xterm.js 的 ITheme,所以任何 iTerm2 / Windows Terminal 配色都能手工贴进来。",
  "settings.io.apply": "应用上面的 JSON",
  "settings.io.download": "导出为文件",
  "settings.io.upload": "从文件导入",
  "settings.io.reset": "恢复全部默认",
  "settings.io.resetConfirm": "把主题、字体、终端设置全部恢复为默认?",
  "settings.io.resetDone": "已恢复默认(主题跟随系统:{name})",
  "settings.io.srcImport": "导入",
  "settings.io.srcFile": "从文件导入",
  "settings.io.parseFail": "{source}失败:不是合法的 JSON({error})",
  "settings.io.ok": "{source}成功",
  "settings.io.okWarn": "{source}成功,但有 {n} 处被修正:",

  /* ---------- 枚举值 ---------- */
  "cursor.block": "方块",
  "cursor.underline": "下划线",
  "cursor.bar": "竖线",
  "cursor.outline": "空心框",
  "cursor.none": "无",

  /* ---------- 单位 ---------- */
  "unit.px": "px",
  "unit.times": "倍",
  "unit.lines": "行",

  /* ---------- 配置校验(config.ts) ---------- */
  "config.warn.notNumber": "{path}: 不是数字,用默认值 {fallback}",
  "config.warn.outOfRange": "{path}: {value} 超出 [{lo}, {hi}],已收敛到 {clamped}",
  "config.warn.notBool": "{path}: 不是布尔值,用默认值 {fallback}",
  "config.warn.notInSet": "{path}: 不是 {allowed} 之一,用默认值 {fallback}",
  "config.warn.badWeight": "{path}: 不是合法字重,用默认值 {fallback}",
  "config.warn.familyNotString": "font.family: 不是字符串,用默认字体栈",
  "config.warn.familyIllegal": "font.family: 含非法字符,已剔除",
  "config.warn.badColor": "{path}: 不是 #rgb/#rrggbb 形式的颜色,已忽略",
  "config.warn.notObject": "配置不是一个 JSON 对象,已全部使用默认值",
  "config.warn.newerVersion": "配置来自更新的版本(v{version} > v{current}),不认得的字段会被忽略",
  "config.warn.themeUnknown": "theme: 目录里没有「{name}」且未内联颜色,退回 {fallback}",
} as const;

export type MsgKey = keyof typeof zh;

const en: Record<MsgKey, string> = {
  "app.settings": "Settings",
  "app.newSession": "New session",
  "app.sessionList": "Sessions",
  "app.kernel": "Local session kernel",
  "app.empty": "No sessions yet. Hit + to start one.",
  "app.backToList": "Back to the session list",
  "session.listTitle": "Sessions",
  "session.open": "Open",
  "app.tokenPrompt": "octoterm token:",

  "conn.connected": "Connected",
  "conn.reconnecting": "Reconnecting",
  "conn.disconnected": "Disconnected",
  "conn.banner.reconnecting": "Reconnecting…",
  "conn.banner.fatal": "Connection failed: {message} — reload the page to enter a token again",

  "session.rename": "Rename",
  "session.kill": "Close session",
  "session.renamePrompt": "New name:",

  /* ---------- agent integration ---------- */
  "agent.state.idle": "Idle",
  "agent.state.thinking": "Thinking",
  "agent.state.working": "Working",
  "agent.state.waiting": "Waiting for you",
  "agent.state.waitingTerminal": "Waiting in the terminal",
  "agent.notice.permission": "wants approval",
  "agent.notice.idle": "idle, waiting on you",
  "agent.notice.input": "needs your input",
  "agent.notice.choice": "waiting for a choice",
  "agent.denyReason": "Reason for denying (optional)",
  "agent.expiresIn": "{n}s left",
  "agent.expired": "expired",
  "agent.unknownTool": "unspecified action",
  "agent.reviewHint": "Read this before deciding — allowing runs it immediately",
  "agent.state.done": "Finished",
  "agent.state.error": "Error",
  "agent.waitingTitle": "An agent is waiting for you",
  "agent.allow": "Allow",
  "agent.deny": "Deny",
  "agent.openSession": "Go to session",
  "agent.gone": "That request has expired",
  "agent.already": "Another device answered it already",
  "agent.failed": "Could not send the answer, try again",
  "settings.tab.agents": "Agents",
  "agent.install": "Install",
  "agent.uninstall": "Uninstall",
  "agent.installed": "Installed",
  "agent.notInstalled": "Not installed",
  "agent.stalePort": "Installed, but pointing at a different port (remote takeover is inactive)",
  "agent.notFound": "No integrable agent found on this machine",
  "agent.disabledHint": "The server has installing disabled (agents.install_enabled in config.toml)",
  "agent.installFailed": "Operation failed",
  "agent.conflictNote": "Another program already has a blocking hook on this event; only status reporting will be installed",
  "agent.writeNote": "Installing edits the agent's own config file (backed up first); uninstalling restores it",
  "agent.preview": "See what changes",
  "agent.previewHide": "Hide",
  "agent.previewFailed": "Could not load the preview",
  "agent.previewAdd": "Hooks written for these events:",
  "agent.previewRemove": "Hooks removed for these events:",
  "agent.previewTelemetryOnly": "Status reporting only, no approval interception (see the conflict above)",
  "agent.activation.codex-hooks-review": "After installing, run /hooks inside Codex to review them — that gate is Codex's own, and only you can clear it",
  "session.empty": "No sessions yet — use + above to start one",

  "ns.filter": "Filter…",
  "ns.filterAria": "Filter launchers",
  "ns.loading": "Loading…",
  "ns.noMatch": "No matching launchers",

  "launcher.defaultName": "Default shell",
  "launcher.defaultDetail": "Chosen by the server",
  "launcher.provider.builtin": "Built-in",
  "launcher.provider.config": "Custom (config.toml)",

  "settings.close": "Close (Esc)",
  "settings.tab.theme": "Theme",
  "settings.tab.font": "Font",
  "settings.tab.terminal": "Terminal",
  "settings.tab.ui": "Interface",
  "settings.tab.io": "Import / Export",

  "settings.theme.search": "Search {n} themes…",
  "settings.theme.searchLoading": "Search themes (loading the full catalog…)",
  "settings.theme.noMatch": "No matching themes",
  "settings.theme.note":
    "Theme data comes from mbadolato/iTerm2-Color-Schemes (MIT). For a palette the catalog doesn't have, edit theme.colors directly under “Import / Export”.",

  "settings.font.family": "Font stack",
  "settings.font.familyHint": "CSS font-family, comma separated",
  "settings.font.available": "✓ Preferred family “{name}” is available",
  "settings.font.missing": "⚠ “{name}” isn't installed; the next family in the stack will be used",
  "settings.font.reset": "Restore the default font stack",
  "settings.font.size": "Size",
  "settings.font.lineHeight": "Line height",
  "settings.font.letterSpacing": "Letter spacing",
  "settings.font.weight": "Weight",
  "settings.font.weightBold": "Bold weight",

  "settings.term.cursorStyle": "Cursor style",
  "settings.term.cursorInactive": "Cursor when unfocused",
  "settings.term.cursorBlink": "Cursor blink",
  "settings.term.scrollback": "Scrollback",
  "settings.term.customGlyphs": "Built-in glyph drawing",
  "settings.term.customGlyphsHint": "Draws box-drawing / powerline itself, no Nerd Font required",
  "settings.term.contrast": "Minimum contrast",
  "settings.term.contrastHint": "1 = leave alone; higher improves readability but rewrites theme colors",
  "settings.term.boldBright": "Bold text in bright colors",

  "settings.ui.language": "Language",
  "settings.ui.languageAuto": "Follow browser",
  "settings.ui.languageHint": "Applies immediately, no reload needed",
  "settings.ui.sidebarPreview": "Session previews in the sidebar",
  "settings.ui.webgl": "WebGL renderer",
  "settings.ui.webglHint": "Falls back to the DOM renderer when off",

  "settings.io.note":
    "The config is the JSON below and can be edited in place. theme.colors is an xterm.js ITheme, so any iTerm2 / Windows Terminal palette can be pasted straight in.",
  "settings.io.apply": "Apply the JSON above",
  "settings.io.download": "Export to a file",
  "settings.io.upload": "Import from a file",
  "settings.io.reset": "Reset everything",
  "settings.io.resetConfirm": "Reset theme, font and terminal settings to their defaults?",
  "settings.io.resetDone": "Reset to defaults (theme follows the system: {name})",
  "settings.io.srcImport": "Import",
  "settings.io.srcFile": "Import from file",
  "settings.io.parseFail": "{source} failed: not valid JSON ({error})",
  "settings.io.ok": "{source} succeeded",
  "settings.io.okWarn": "{source} succeeded, with {n} field(s) corrected:",

  "cursor.block": "Block",
  "cursor.underline": "Underline",
  "cursor.bar": "Bar",
  "cursor.outline": "Outline",
  "cursor.none": "None",

  "unit.px": "px",
  "unit.times": "×",
  "unit.lines": "lines",

  "config.warn.notNumber": "{path}: not a number, using the default {fallback}",
  "config.warn.outOfRange": "{path}: {value} is outside [{lo}, {hi}], clamped to {clamped}",
  "config.warn.notBool": "{path}: not a boolean, using the default {fallback}",
  "config.warn.notInSet": "{path}: not one of {allowed}, using the default {fallback}",
  "config.warn.badWeight": "{path}: not a valid font weight, using the default {fallback}",
  "config.warn.familyNotString": "font.family: not a string, using the default font stack",
  "config.warn.familyIllegal": "font.family: illegal characters removed",
  "config.warn.badColor": "{path}: not a #rgb/#rrggbb color, ignored",
  "config.warn.notObject": "Config is not a JSON object; every value fell back to its default",
  "config.warn.newerVersion":
    "Config comes from a newer version (v{version} > v{current}); unknown fields are ignored",
  "config.warn.themeUnknown":
    "theme: the catalog has no “{name}” and no colors were inlined, falling back to {fallback}",
};

const CATALOGS: Record<Locale, Record<MsgKey, string>> = { "zh-CN": zh, en };

/**
 * 当前语言。默认 en 而不是 zh-CN:这是探测不出结果时的兜底,而项目对外的
 * 第一语言是英文(见 README.md / README-cnzh.md 的主次关系)。正常路径下
 * main.ts 在首帧渲染前就按浏览器偏好把它设好了。
 */
let current: Locale = "en";

type Listener = () => void;
const listeners = new Set<Listener>();

export function getLocale(): Locale {
  return current;
}

/** 当前语言的 BCP 47 标签,给 `<html lang>` 和 `toLocaleString()` 用。 */
export function localeTag(): string {
  return LOCALE_TAGS[current];
}

/** 切换语言并通知所有订阅者重绘。同一语言重复设置是空操作。 */
export function setLocale(locale: Locale): void {
  if (locale === current) return;
  current = locale;
  for (const fn of [...listeners]) fn();
}

/** 订阅语言变更。返回取消订阅的函数。 */
export function subscribe(fn: Listener): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/**
 * 把「偏好 + 浏览器语言列表」解析成一个受支持的语言。纯函数,便于测试。
 *
 * 只比主子标签:zh-TW / zh-Hant 都归到 zh-CN —— 给繁体用户一份简体界面,
 * 比甩一份英文界面更接近他们要的东西。等真加了 zh-TW 词条再细分。
 */
export function resolveLocale(pref: LocalePref, navLangs: readonly string[]): Locale {
  if (pref !== "auto" && (LOCALES as readonly string[]).includes(pref)) return pref;
  for (const raw of navLangs) {
    const primary = String(raw).toLowerCase().split("-")[0];
    const hit = LOCALES.find((l) => l.toLowerCase().split("-")[0] === primary);
    if (hit) return hit;
  }
  return "en";
}

/** 读浏览器语言偏好。这是这个模块里唯一碰宿主环境的地方。 */
export function navigatorLanguages(): readonly string[] {
  try {
    const nav = globalThis.navigator;
    if (!nav) return [];
    return nav.languages?.length ? nav.languages : nav.language ? [nav.language] : [];
  } catch {
    return [];
  }
}

/** `{name}` 占位符替换。没给到的占位符原样留下,便于一眼看出漏传了什么。 */
export function t(key: MsgKey, params?: Record<string, string | number>): string {
  const raw = CATALOGS[current][key] ?? zh[key] ?? key;
  if (!params) return raw;
  return raw.replace(/\{(\w+)\}/g, (m, name: string) =>
    Object.prototype.hasOwnProperty.call(params, name) ? String(params[name]) : m,
  );
}

/** 测试用:拿到某个语言的完整词条表,用来做键位对齐检查。 */
export function catalog(locale: Locale): Record<string, string> {
  return CATALOGS[locale];
}
