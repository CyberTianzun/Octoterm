/**
 * 把终端主题铺到终端以外的界面上。
 *
 * style.css 里那套 --bg/--panel/--text/--accent 原本是写死的 TokyoNight。这里
 * 从 ITheme 里**派生**出同一组变量写到 :root 的内联样式上,于是换主题时侧边栏、
 * 按钮、分隔线一起跟着换,而不是只有中间那块终端变了色。
 *
 * 派生规则是「前景色按不同比例混进背景色」——比硬编码一套灰阶更稳:亮色主题
 * (Solarized Light)和暗色主题用同一套公式都能得到合理的层次。
 */
import type { ITheme } from "@xterm/xterm";
import type { OctoConfig } from "./config";

/** 这些变量由主题接管;关掉「界面跟随主题」时全部移除,让 style.css 的默认值生效。 */
const VARS = ["--bg", "--panel", "--panel-2", "--line", "--text", "--dim", "--accent", "--danger"];

interface Rgb {
  r: number;
  g: number;
  b: number;
}

/** 解析 #rgb / #rgba / #rrggbb / #rrggbbaa。config.ts 已经保证了只有这几种形态。 */
function parseHex(hex: string | undefined): Rgb | null {
  if (!hex || hex[0] !== "#") return null;
  const h = hex.slice(1);
  const short = h.length === 3 || h.length === 4;
  if (!short && h.length !== 6 && h.length !== 8) return null;
  const at = (i: number) =>
    short ? parseInt(h[i] + h[i], 16) : parseInt(h.slice(i * 2, i * 2 + 2), 16);
  const [r, g, b] = [at(0), at(1), at(2)];
  return Number.isNaN(r + g + b) ? null : { r, g, b };
}

const toHex = (c: Rgb) =>
  "#" + [c.r, c.g, c.b].map((v) => Math.round(v).toString(16).padStart(2, "0")).join("");

/** a 混进 b,t=0 全是 b,t=1 全是 a。 */
const mix = (a: Rgb, b: Rgb, t: number): Rgb => ({
  r: b.r + (a.r - b.r) * t,
  g: b.g + (a.g - b.g) * t,
  b: b.b + (a.b - b.b) * t,
});

/** 相对亮度(sRGB 加权近似),只用来判断亮色/暗色主题。 */
const luminance = (c: Rgb) => (0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b) / 255;

/** WCAG 相对亮度(带 gamma 校正),用于算对比度 —— 和上面那个粗略版不是一回事。 */
function wcagLuminance(c: Rgb): number {
  const ch = (v: number) => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return 0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b);
}

function contrast(a: Rgb, b: Rgb): number {
  const [hi, lo] = [wcagLuminance(a), wcagLuminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** WCAG 对 UI 组件 / 大字的最低对比度。低于这个值的强调色在界面上基本看不见。 */
const MIN_UI_CONTRAST = 3;

/**
 * 在「常规色 / 亮色变体 / 前景色」里挑一个能在背景上看清的。
 *
 * 优先用常规变体(那是主题作者的本意),只有它在背景上达不到最低对比度时才升级到
 * 亮色变体。必要的:有些主题的常规蓝在自己的背景上对比度只有 2 出头(iTerm2
 * Default 的 #2225c4 配纯黑是 2.13),直接拿来当 --accent 就是一片糊。
 */
function readableAccent(normal: string | undefined, bright: string | undefined, bg: Rgb, fg: Rgb): string {
  const candidates = [normal, bright]
    .map((h) => ({ hex: h, rgb: parseHex(h) }))
    .filter((c): c is { hex: string; rgb: Rgb } => c.rgb !== null);
  if (candidates.length === 0) return toHex(fg);
  const first = candidates[0];
  if (contrast(first.rgb, bg) >= MIN_UI_CONTRAST) return first.hex;
  // 常规色不够用,退而取对比度最高的那个候选(含常规色本身)
  return candidates.reduce((best, c) => (contrast(c.rgb, bg) > contrast(best.rgb, bg) ? c : best)).hex;
}

export function isLightTheme(theme: ITheme): boolean {
  const bg = parseHex(theme.background);
  return bg !== null && luminance(bg) > 0.5;
}

export interface UiColors {
  vars: Record<string, string>;
  colorScheme: "light" | "dark";
}

/**
 * 从主题派生出整套界面变量。纯函数,不碰 DOM —— 亮色/暗色两条分支正是最容易
 * 悄悄产出「白底白字」的地方,得能单测。
 *
 * 主题连前景或背景都没给时返回 null,表示「不如维持 style.css 的原样」。
 */
export function deriveUiColors(theme: ITheme): UiColors | null {
  const bg = parseHex(theme.background);
  const fg = parseHex(theme.foreground);
  if (!bg || !fg) return null;

  const light = luminance(bg) > 0.5;
  return {
    colorScheme: light ? "light" : "dark",
    vars: {
      "--bg": toHex(bg),
      // 侧边栏要能跟终端区分开:往背景里掺一点前景色。亮色主题掺得少一些,
      // 因为同样的比例在浅底上看起来更脏。
      "--panel": toHex(mix(fg, bg, light ? 0.05 : 0.07)),
      "--panel-2": toHex(mix(fg, bg, light ? 0.11 : 0.14)),
      "--line": toHex(mix(fg, bg, light ? 0.2 : 0.22)),
      "--text": toHex(fg),
      "--dim": toHex(mix(fg, bg, 0.55)),
      "--accent": readableAccent(theme.blue, theme.brightBlue, bg, fg),
      "--danger": readableAccent(theme.red, theme.brightRed, bg, fg),
    },
  };
}

/**
 * 应用界面配色。
 *
 * 只写 :root 的内联样式,不动 style.css —— 主题派生不出配色时(缺前景或背景)
 * 把这几个变量删掉就回到原样了,不需要维护一份「默认值」的副本。
 */
export function applyUiColors(cfg: OctoConfig): void {
  const root = document.documentElement.style;
  const derived = deriveUiColors(cfg.theme.colors);
  if (!derived) {
    for (const v of VARS) root.removeProperty(v);
    root.removeProperty("color-scheme");
    return;
  }
  for (const [k, v] of Object.entries(derived.vars)) root.setProperty(k, v);
  // 让原生滚动条 / 表单控件跟着走,否则亮色主题下会出现深色滚动条
  root.setProperty("color-scheme", derived.colorScheme);
}
