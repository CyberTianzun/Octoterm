// 从 mbadolato/iTerm2-Color-Schemes(MIT)生成主题数据,vendored 进仓库。
//
// 这是**开发期一次性脚本**,不在 `npm run build` 里 —— 构建不该依赖网络,主题
// 数据也不该随上游漂移。想更新主题表时手动跑 `npm run gen:themes`。
//
// 上游的 windowsterminal/*.json 和 xterm.js 的 ITheme 差三个键名,见 TO_ITHEME。
import { execFileSync } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = "https://github.com/mbadolato/iTerm2-Color-Schemes.git";
const SUBDIR = "windowsterminal";
const OUT = join(dirname(fileURLToPath(import.meta.url)), "..", "src", "themes");

/** 上游键名 -> ITheme 键名。未列出的键名两边一致,直接透传。 */
const TO_ITHEME = {
  purple: "magenta",
  brightPurple: "brightMagenta",
  cursorColor: "cursor",
};
/** ITheme 认得的键,其余(比如上游的 "name")丢弃。 */
const ITHEME_KEYS = new Set([
  "foreground", "background", "cursor", "cursorAccent",
  "selectionBackground", "selectionForeground", "selectionInactiveBackground",
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "brightBlack", "brightRed", "brightGreen", "brightYellow",
  "brightBlue", "brightMagenta", "brightCyan", "brightWhite",
]);

/**
 * 上游目录里没有、我们自己补的主题。会并进 catalog,和上游的一视同仁。
 *
 * 每一条都必须写清 provenance:主题的 hex 值不能靠印象填,得指得出出处。
 */
const EXTRA_THEMES = {
  // VS Code v1.109 起的默认暗色主题「2026 Dark」在它**集成终端**里的样子。
  //
  // 出处(2026-08 从 microsoft/vscode main 分支核对):
  //   background / selectionBackground
  //     extensions/theme-defaults/themes/2026-dark.json 的
  //     terminal.background / terminal.selectionBackground
  //   foreground / cursor
  //     同一文件,沿 include 链 2026-dark -> dark_modern -> dark_plus -> dark_vs
  //     解析出的 terminal.foreground / terminalCursor.foreground
  //   16 色 ANSI
  //     src/vs/workbench/contrib/terminal/common/terminalColorRegistry.ts 里
  //     ansiColorMap 的 dark 默认值 —— VS Code 的默认主题不覆盖 ANSI,继承链上
  //     一个 terminal.ansi* 都没有,所以终端里生效的就是这组内置默认值。
  //
  // 这和社区那些叫「2026 Dark」的终端移植不一样:它们多半拿 editor.background
  // 和语法高亮色来凑,那是编辑器的观感,不是 VS Code 终端的观感,而且 bright
  // 色往往和常规色重复,TUI 里会少一档层次。
  "2026 Dark": {
    background: "#191a1b",
    foreground: "#cccccc",
    cursor: "#bfbfbf",
    selectionBackground: "#3994bc33",
    black: "#000000",
    red: "#cd3131",
    green: "#0dbc79",
    yellow: "#e5e510",
    blue: "#2472c8",
    magenta: "#bc3fbc",
    cyan: "#11a8cd",
    white: "#e5e5e5",
    brightBlack: "#666666",
    brightRed: "#f14c4c",
    brightGreen: "#23d18b",
    brightYellow: "#f5f543",
    brightBlue: "#3b8eea",
    brightMagenta: "#d670d6",
    brightCyan: "#29b8db",
    brightWhite: "#e5e5e5",
  },

  // 「2026 Light」,同上,链是 2026-light -> light_modern -> light_plus -> light_vs,
  // ANSI 取 ansiColorMap 的 light 默认值。
  //
  // 一处和 Dark 不同的地方:这条链里**没有** terminal.background。registry 里
  // terminal.background 注册的默认值是 null,即回落到面板背景,所以这里取
  // panel.background #fafafd。这个推断在 Dark 上可以自证:Dark 显式写了
  // terminal.background #191a1b,而它的 panel.background 恰好也是 #191a1b。
  //
  // 另注:VS Code 的 light ANSI 默认值里 red/blue/magenta/cyan 四组的 bright 和
  // 常规色是相同的 —— 这是上游本来的样子,不是我们抄漏了。
  "2026 Light": {
    background: "#fafafd",
    foreground: "#3b3b3b",
    cursor: "#202020",
    selectionBackground: "#0069cc26",
    black: "#000000",
    red: "#cd3131",
    green: "#107c10",
    yellow: "#949800",
    blue: "#0451a5",
    magenta: "#bc05bc",
    cyan: "#0598bc",
    white: "#555555",
    brightBlack: "#666666",
    brightRed: "#cd3131",
    brightGreen: "#14ce14",
    brightYellow: "#b5ba00",
    brightBlue: "#0451a5",
    brightMagenta: "#bc05bc",
    brightCyan: "#0598bc",
    brightWhite: "#a5a5a5",
  },
};

/**
 * 首次打开时按系统的 prefers-color-scheme 二选一。两个都必须在 BUILTIN 里
 * (首屏就要用,不能等全量目录 fetch 回来)—— 下面有校验。
 */
const DEFAULT_THEMES = { dark: "2026 Dark", light: "2026 Light" };

/** 编进 bundle 的默认主题:覆盖亮/暗、覆盖几个主流生态,首屏不用等网络。 */
const BUILTIN = [
  "2026 Dark",
  "2026 Light",
  "JetBrains Islands Dark",
  "TokyoNight",
  "Dracula",
  "Nord",
  "Gruvbox Dark",
  "Catppuccin Mocha",
  "iTerm2 Solarized Dark",
  "iTerm2 Solarized Light",
  "One Half Light",
];

function sparseCheckout() {
  const dir = mkdtempSync(join(tmpdir(), "octoterm-themes-"));
  const git = (...args) => execFileSync("git", ["-C", dir, ...args], { stdio: ["ignore", "pipe", "inherit"] });
  execFileSync("git", ["clone", "--depth", "1", "--filter=blob:none", "--sparse", REPO, dir], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  git("sparse-checkout", "set", SUBDIR);
  return dir;
}

function toITheme(raw) {
  const out = {};
  for (const [k, v] of Object.entries(raw)) {
    const key = TO_ITHEME[k] ?? k;
    // 统一小写:上游本来就是小写,EXTRA_THEMES 抄来的源可能是大写,
    // 混在一起会让 catalog.json 的 diff 变噪音
    if (ITHEME_KEYS.has(key)) out[key] = String(v).toLowerCase();
  }
  return out;
}

const repo = sparseCheckout();
let catalog;
try {
  const files = readdirSync(join(repo, SUBDIR)).filter((f) => f.endsWith(".json")).sort();
  catalog = {};
  for (const f of files) {
    const raw = JSON.parse(readFileSync(join(repo, SUBDIR, f), "utf8"));
    catalog[raw.name ?? f.replace(/\.json$/, "")] = toITheme(raw);
  }
} finally {
  rmSync(repo, { recursive: true, force: true });
}

for (const [name, theme] of Object.entries(EXTRA_THEMES)) {
  // 撞名说明上游后来自己收录了这个主题 —— 那时应该考虑删掉我们这份,
  // 而不是继续用本地版本悄悄盖掉上游。
  if (name in catalog) console.warn(`! 「${name}」上游已收录,EXTRA_THEMES 里的同名条目正在覆盖它`);
  catalog[name] = toITheme(theme);
}

const missing = BUILTIN.filter((n) => !(n in catalog));
if (missing.length) throw new Error(`BUILTIN 里的主题上游没有: ${missing.join(", ")}`);

// 默认主题必须编进 bundle:首屏(localStorage 还没有配置时)就要用,
// 那会儿全量目录连请求都还没发出去。
const notBuiltin = Object.entries(DEFAULT_THEMES).filter(([, n]) => !BUILTIN.includes(n));
if (notBuiltin.length) {
  throw new Error(
    `DEFAULT_THEMES 必须同时出现在 BUILTIN 里: ${notBuiltin.map(([k, n]) => `${k}=${n}`).join(", ")}`,
  );
}

mkdirSync(OUT, { recursive: true });
writeFileSync(join(OUT, "catalog.json"), JSON.stringify(catalog));

const builtin = Object.fromEntries(BUILTIN.map((n) => [n, catalog[n]]));
writeFileSync(
  join(OUT, "builtin.ts"),
  `// 由 scripts/gen-themes.mjs 生成,不要手改。源:mbadolato/iTerm2-Color-Schemes (MIT)\n` +
    `import type { ITheme } from "@xterm/xterm";\n\n` +
    `/** 首次打开时按系统 prefers-color-scheme 二选一,见 config.ts 的 defaultConfig。 */\n` +
    `export const DEFAULT_THEMES = ${JSON.stringify(DEFAULT_THEMES)} as const;\n\n` +
    `export const BUILTIN_THEMES: Record<string, ITheme> = ${JSON.stringify(builtin, null, 2)};\n`,
);

console.log(`themes: ${Object.keys(catalog).length} -> src/themes/catalog.json`);
console.log(`builtin: ${BUILTIN.length} -> src/themes/builtin.ts`);
