/**
 * 客户端外观配置:主题 / 字体 / 终端行为。
 *
 * 三条约束决定了这个模块的形状:
 *
 * 1. **纯客户端**。docs/protocol.md 的 G3 已经定了「配置不上线协商」,外观是本
 *    端表现,不进协议、不进服务端,只活在 localStorage 里。
 * 2. **纯函数,不碰 DOM**(localStorage 那两个函数除外),这样能被 node:test 直接
 *    跑,不用起浏览器。
 * 3. **导入的 JSON 是不可信输入**。颜色最终会被写进 CSS 自定义属性、字体名会
 *    进 font-family,所以 sanitize 不是防御性编程洁癖,是必须的:见 COLOR_RE
 *    和 sanitizeFontFamily。sanitize 永不抛异常 —— 坏字段退回默认值并记一条
 *    warning,让用户导入一个半坏的文件时仍然能进到可用状态。
 */
import type { ITheme, ITerminalOptions, FontWeight } from "@xterm/xterm";
import { BUILTIN_THEMES, DEFAULT_THEMES } from "./themes/builtin";
import { LOCALES, type LocalePref, t } from "./i18n";

export const CONFIG_VERSION = 1;
export const STORAGE_KEY = "octoterm-config";

export interface OctoConfig {
  version: number;
  theme: {
    /** 主题名。只是标签 —— 真正生效的是 colors,这样导出的 JSON 自带全部颜色, */
    /** 换一台机器/换一个客户端导入时不需要同一份主题目录。 */
    name: string;
    colors: ITheme;
  };
  font: {
    family: string;
    size: number;
    weight: FontWeight;
    weightBold: FontWeight;
    lineHeight: number;
    letterSpacing: number;
  };
  terminal: {
    cursorStyle: "block" | "underline" | "bar";
    cursorBlink: boolean;
    cursorInactiveStyle: "outline" | "block" | "bar" | "underline" | "none";
    scrollback: number;
    customGlyphs: boolean;
    minimumContrastRatio: number;
    drawBoldTextInBrightColors: boolean;
  };
  ui: {
    /** 界面语言。"auto" = 跟随浏览器,见 i18n.resolveLocale。 */
    locale: LocalePref;
    /** 用主题色驱动整个界面(侧边栏等)的 CSS 变量,而不是只染终端那块。 */
    followThemeColors: boolean;
    /** 侧边栏里的小终端预览。关掉能省掉每个会话一个 Terminal 实例。 */
    sidebarPreview: boolean;
    /** WebGL 渲染器。关掉回落到 DOM 渲染器。 */
    webgl: boolean;
  };
}

/** ITheme 的全部颜色键(extendedAnsi 单独处理:它是数组)。 */
export const THEME_COLOR_KEYS = [
  "foreground", "background", "cursor", "cursorAccent",
  "selectionBackground", "selectionForeground", "selectionInactiveBackground",
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "brightBlack", "brightRed", "brightGreen", "brightYellow",
  "brightBlue", "brightMagenta", "brightCyan", "brightWhite",
] as const;

/** 只收十六进制。这些字符串会被原样插进 CSS 自定义属性,放开成任意 CSS 颜色
 *  就等于开了一个字符串注入口子;而现实里的主题文件本来就全是 hex。 */
const COLOR_RE = /^#(?:[0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/;

const FONT_WEIGHTS: readonly string[] = [
  "normal", "bold", "100", "200", "300", "400", "500", "600", "700", "800", "900",
];

export const DEFAULT_FONT_FAMILY =
  'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, "DejaVu Sans Mono", monospace';

/**
 * 默认配置。`prefersDark` 决定默认主题取 2026 Dark 还是 2026 Light。
 *
 * 参数化而不是在这里读 matchMedia,是为了保住这个模块「纯函数、不碰 DOM」的
 * 性质(见文件顶部)——系统偏好由调用方在副作用边界上读,见 systemPrefersDark。
 */
export function defaultConfig(prefersDark = true): OctoConfig {
  const themeName = prefersDark ? DEFAULT_THEMES.dark : DEFAULT_THEMES.light;
  return {
    version: CONFIG_VERSION,
    theme: { name: themeName, colors: { ...BUILTIN_THEMES[themeName] } },
    font: {
      family: DEFAULT_FONT_FAMILY,
      size: 14,
      weight: "normal",
      weightBold: "bold",
      lineHeight: 1.0,
      letterSpacing: 0,
    },
    terminal: {
      cursorStyle: "block",
      cursorBlink: true,
      cursorInactiveStyle: "outline",
      scrollback: 5000,
      customGlyphs: true,
      minimumContrastRatio: 1,
      drawBoldTextInBrightColors: true,
    },
    ui: { locale: "auto", followThemeColors: true, sidebarPreview: true, webgl: true },
  };
}

/* ---------- 逐字段校验:每个都在坏输入时回退到 fallback 并记一条 warning ---------- */

type Warn = (msg: string) => void;

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function num(v: unknown, lo: number, hi: number, fallback: number, path: string, warn: Warn): number {
  if (typeof v !== "number" || !Number.isFinite(v)) {
    if (v !== undefined) warn(t("config.warn.notNumber", { path, fallback }));
    return fallback;
  }
  const clamped = Math.min(hi, Math.max(lo, v));
  if (clamped !== v) warn(t("config.warn.outOfRange", { path, value: v, lo, hi, clamped }));
  return clamped;
}

function int(v: unknown, lo: number, hi: number, fallback: number, path: string, warn: Warn): number {
  return Math.round(num(v, lo, hi, fallback, path, warn));
}

function bool(v: unknown, fallback: boolean, path: string, warn: Warn): boolean {
  if (typeof v === "boolean") return v;
  if (v !== undefined) warn(t("config.warn.notBool", { path, fallback: String(fallback) }));
  return fallback;
}

function pick<T extends string>(v: unknown, allowed: readonly T[], fallback: T, path: string, warn: Warn): T {
  if (typeof v === "string" && (allowed as readonly string[]).includes(v)) return v as T;
  if (v !== undefined) warn(t("config.warn.notInSet", { path, allowed: allowed.join("/"), fallback }));
  return fallback;
}

function fontWeight(v: unknown, fallback: FontWeight, path: string, warn: Warn): FontWeight {
  if (typeof v === "number" && Number.isFinite(v)) return Math.min(900, Math.max(1, Math.round(v)));
  if (typeof v === "string" && FONT_WEIGHTS.includes(v)) return v as FontWeight;
  if (v !== undefined) warn(t("config.warn.badWeight", { path, fallback }));
  return fallback;
}

/** font-family 会进 CSS。只放行字母数字、空格和 CSS 标识符里合法的少数标点,
 *  挡掉 `;{}()` 等能撑破声明的字符。截断到 200 字符防止病态输入。 */
export function sanitizeFontFamily(v: unknown, fallback: string, warn: Warn): string {
  if (typeof v !== "string") {
    if (v !== undefined) warn(t("config.warn.familyNotString"));
    return fallback;
  }
  const cleaned = v.replace(/[^A-Za-z0-9 ,._'"-]/g, "").trim().slice(0, 200);
  if (cleaned !== v.trim()) warn(t("config.warn.familyIllegal"));
  return cleaned === "" ? fallback : cleaned;
}

function color(v: unknown, path: string, warn: Warn): string | undefined {
  if (typeof v === "string" && COLOR_RE.test(v)) return v;
  if (v !== undefined) warn(t("config.warn.badColor", { path }));
  return undefined;
}

/** 把任意输入收敛成一个 ITheme。空对象是合法的(xterm 会用它自己的默认色)。 */
export function sanitizeTheme(input: unknown, warn: Warn = () => {}): ITheme {
  const src = isObj(input) ? input : {};
  const out: Record<string, unknown> = {};
  for (const k of THEME_COLOR_KEYS) {
    const c = color(src[k], `theme.colors.${k}`, warn);
    if (c !== undefined) out[k] = c;
  }
  if (Array.isArray(src.extendedAnsi)) {
    const ext = src.extendedAnsi
      .map((c, i) => color(c, `theme.colors.extendedAnsi[${i}]`, warn))
      .filter((c): c is string => c !== undefined);
    if (ext.length > 0) out.extendedAnsi = ext.slice(0, 240);
  }
  return out as ITheme;
}

/**
 * 把任意输入收敛成一个可用的 OctoConfig。永不抛异常。
 *
 * `resolveTheme` 用来处理「只给了主题名、没给颜色」的导入(手写配置的常见形态):
 * 拿名字去查目录。查不到就退回默认主题。
 */
export function sanitizeConfig(
  input: unknown,
  opts: { resolveTheme?: (name: string) => ITheme | undefined; warn?: Warn } = {},
): OctoConfig {
  const warn = opts.warn ?? (() => {});
  const resolve = opts.resolveTheme ?? ((n: string) => BUILTIN_THEMES[n]);
  const d = defaultConfig();
  const src = isObj(input) ? input : {};
  if (!isObj(input)) warn(t("config.warn.notObject"));

  if (typeof src.version === "number" && src.version > CONFIG_VERSION) {
    warn(t("config.warn.newerVersion", { version: src.version, current: CONFIG_VERSION }));
  }

  const themeSrc = isObj(src.theme) ? src.theme : {};
  const name =
    typeof themeSrc.name === "string" && themeSrc.name.trim() !== ""
      ? themeSrc.name.trim().slice(0, 80)
      : d.theme.name;
  let colors: ITheme;
  if (isObj(themeSrc.colors)) {
    colors = sanitizeTheme(themeSrc.colors, warn);
  } else {
    // 只给了名字:去目录里查。这样手写 {"theme":{"name":"Dracula"}} 也能用。
    const found = resolve(name);
    if (found) {
      colors = sanitizeTheme(found, warn);
    } else {
      warn(t("config.warn.themeUnknown", { name, fallback: d.theme.name }));
      return { ...d, ...sanitizeRest(src, d, warn) };
    }
  }

  return { version: CONFIG_VERSION, theme: { name, colors }, ...sanitizeRest(src, d, warn) };
}

function sanitizeRest(
  src: Record<string, unknown>,
  d: OctoConfig,
  warn: Warn,
): Pick<OctoConfig, "font" | "terminal" | "ui"> {
  const f = isObj(src.font) ? src.font : {};
  const t = isObj(src.terminal) ? src.terminal : {};
  const u = isObj(src.ui) ? src.ui : {};
  return {
    font: {
      family: sanitizeFontFamily(f.family, d.font.family, warn),
      size: int(f.size, 6, 48, d.font.size, "font.size", warn),
      weight: fontWeight(f.weight, d.font.weight, "font.weight", warn),
      weightBold: fontWeight(f.weightBold, d.font.weightBold, "font.weightBold", warn),
      lineHeight: num(f.lineHeight, 0.8, 3, d.font.lineHeight, "font.lineHeight", warn),
      letterSpacing: num(f.letterSpacing, -5, 10, d.font.letterSpacing, "font.letterSpacing", warn),
    },
    terminal: {
      cursorStyle: pick(t.cursorStyle, ["block", "underline", "bar"] as const, d.terminal.cursorStyle, "terminal.cursorStyle", warn),
      cursorBlink: bool(t.cursorBlink, d.terminal.cursorBlink, "terminal.cursorBlink", warn),
      cursorInactiveStyle: pick(t.cursorInactiveStyle, ["outline", "block", "bar", "underline", "none"] as const, d.terminal.cursorInactiveStyle, "terminal.cursorInactiveStyle", warn),
      scrollback: int(t.scrollback, 0, 200_000, d.terminal.scrollback, "terminal.scrollback", warn),
      customGlyphs: bool(t.customGlyphs, d.terminal.customGlyphs, "terminal.customGlyphs", warn),
      minimumContrastRatio: num(t.minimumContrastRatio, 1, 21, d.terminal.minimumContrastRatio, "terminal.minimumContrastRatio", warn),
      drawBoldTextInBrightColors: bool(t.drawBoldTextInBrightColors, d.terminal.drawBoldTextInBrightColors, "terminal.drawBoldTextInBrightColors", warn),
    },
    ui: {
      locale: pick(u.locale, ["auto", ...LOCALES] as const, d.ui.locale, "ui.locale", warn),
      followThemeColors: bool(u.followThemeColors, d.ui.followThemeColors, "ui.followThemeColors", warn),
      sidebarPreview: bool(u.sidebarPreview, d.ui.sidebarPreview, "ui.sidebarPreview", warn),
      webgl: bool(u.webgl, d.ui.webgl, "ui.webgl", warn),
    },
  };
}

/* ---------- 与 xterm 的接口 ---------- */

/** 配置 -> xterm 选项。赋给 `term.options` 即时生效,不需要重建 Terminal。 */
export function toTerminalOptions(cfg: OctoConfig): ITerminalOptions {
  return {
    theme: cfg.theme.colors,
    fontFamily: cfg.font.family,
    fontSize: cfg.font.size,
    fontWeight: cfg.font.weight,
    fontWeightBold: cfg.font.weightBold,
    lineHeight: cfg.font.lineHeight,
    letterSpacing: cfg.font.letterSpacing,
    cursorStyle: cfg.terminal.cursorStyle,
    cursorBlink: cfg.terminal.cursorBlink,
    cursorInactiveStyle: cfg.terminal.cursorInactiveStyle,
    scrollback: cfg.terminal.scrollback,
    customGlyphs: cfg.terminal.customGlyphs,
    minimumContrastRatio: cfg.terminal.minimumContrastRatio,
    drawBoldTextInBrightColors: cfg.terminal.drawBoldTextInBrightColors,
  };
}

/** 侧边栏预览:共享配色和字形,但字号钉死(要塞进 96px 高的小格子),
 *  也不需要滚动缓冲和光标。 */
export function toPreviewOptions(cfg: OctoConfig): ITerminalOptions {
  return {
    theme: cfg.theme.colors,
    fontFamily: cfg.font.family,
    fontSize: 5,
    lineHeight: 1,
    letterSpacing: 0,
    customGlyphs: cfg.terminal.customGlyphs,
    scrollback: 0,
    disableStdin: true,
    cursorInactiveStyle: "none",
  };
}

/* ---------- 序列化 ---------- */

export function exportConfigJson(cfg: OctoConfig): string {
  return JSON.stringify(cfg, null, 2) + "\n";
}

export interface ImportResult {
  config: OctoConfig;
  warnings: string[];
}

/** 解析导入的 JSON 文本。JSON 本身坏掉才抛 —— 语义上的坏字段走 warnings。 */
export function importConfigJson(
  text: string,
  resolveTheme?: (name: string) => ITheme | undefined,
): ImportResult {
  const parsed = JSON.parse(text);
  const warnings: string[] = [];
  const config = sanitizeConfig(parsed, { resolveTheme, warn: (m) => warnings.push(m) });
  return { config, warnings };
}

/* ---------- 浏览器环境 ---------- */

/** 系统是否偏好深色。matchMedia 缺席(老浏览器 / 非 DOM 环境)时按深色算。 */
export function systemPrefersDark(): boolean {
  try {
    return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? true;
  } catch {
    return true;
  }
}

/** 跟随系统亮暗的默认配置。首次打开和「恢复全部默认」都走这里。 */
export function systemDefaultConfig(): OctoConfig {
  return defaultConfig(systemPrefersDark());
}

/* ---------- localStorage ---------- */

/**
 * 读持久化配置。隐私模式 / 配额异常 / 存了个坏值,一律退回默认配置。
 *
 * 没有存过配置时按系统亮暗挑默认主题。注意这一步**不写回** localStorage:
 * 在用户真正改过一次设置之前,主题就一直跟着系统走;一旦改过(哪怕改的是字号),
 * 整份配置被持久化,此后系统再切换亮暗也不会覆盖用户自己的选择。
 */
export function loadConfig(resolveTheme?: (name: string) => ITheme | undefined): OctoConfig {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null) return systemDefaultConfig();
    return sanitizeConfig(JSON.parse(raw), { resolveTheme });
  } catch {
    return systemDefaultConfig();
  }
}

/** 写持久化配置。写不进去(隐私模式/配额满)不是致命错误,当次会话照常工作。 */
export function saveConfig(cfg: OctoConfig): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(cfg));
  } catch {
    /* 忽略 */
  }
}
