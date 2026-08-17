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

/** 编进 bundle 的默认主题:覆盖亮/暗、覆盖几个主流生态,首屏不用等网络。 */
const BUILTIN = [
  "JetBrains Islands Dark", // 默认(数组第一项即 DEFAULT_THEME_NAME)
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
    if (ITHEME_KEYS.has(key)) out[key] = v;
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

const missing = BUILTIN.filter((n) => !(n in catalog));
if (missing.length) throw new Error(`BUILTIN 里的主题上游没有: ${missing.join(", ")}`);

mkdirSync(OUT, { recursive: true });
writeFileSync(join(OUT, "catalog.json"), JSON.stringify(catalog));

const builtin = Object.fromEntries(BUILTIN.map((n) => [n, catalog[n]]));
writeFileSync(
  join(OUT, "builtin.ts"),
  `// 由 scripts/gen-themes.mjs 生成,不要手改。源:mbadolato/iTerm2-Color-Schemes (MIT)\n` +
    `import type { ITheme } from "@xterm/xterm";\n\n` +
    `export const DEFAULT_THEME_NAME = ${JSON.stringify(BUILTIN[0])};\n\n` +
    `export const BUILTIN_THEMES: Record<string, ITheme> = ${JSON.stringify(builtin, null, 2)};\n`,
);

console.log(`themes: ${Object.keys(catalog).length} -> src/themes/catalog.json`);
console.log(`builtin: ${BUILTIN.length} -> src/themes/builtin.ts`);
