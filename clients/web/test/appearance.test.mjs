import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { execSync } from "node:child_process";

// deriveUiColors 是纯函数;applyUiColors 里那句 document 只在函数体内,import
// 本身不碰 DOM,所以能直接在 node 里跑。
const here = dirname(fileURLToPath(import.meta.url));
execSync("npx esbuild src/appearance.ts --bundle --format=esm --outfile=test/.appearance.build.mjs", {
  cwd: join(here, ".."),
});
// 内置主题表也转一份:下面那条「每个内置主题都够对比度」的测试要拿真数据跑
execSync("npx esbuild src/themes/builtin.ts --bundle --format=esm --outfile=test/.builtin.build.mjs", {
  cwd: join(here, ".."),
});
const { deriveUiColors, isLightTheme } = await import("./.appearance.build.mjs");

const TOKYO = { background: "#1a1b26", foreground: "#c0caf5", blue: "#7aa2f7", red: "#f7768e" };
const SOLARIZED_LIGHT = { background: "#fdf6e3", foreground: "#657b83", blue: "#268bd2", red: "#dc322f" };

const sum = (hex) => {
  const h = hex.slice(1);
  return parseInt(h.slice(0, 2), 16) + parseInt(h.slice(2, 4), 16) + parseInt(h.slice(4, 6), 16);
};

test("暗色主题:面板比背景亮,color-scheme 为 dark", () => {
  const { vars, colorScheme } = deriveUiColors(TOKYO);
  assert.equal(colorScheme, "dark");
  assert.equal(vars["--bg"], "#1a1b26");
  assert.equal(vars["--text"], "#c0caf5");
  assert.ok(sum(vars["--panel"]) > sum(vars["--bg"]), "--panel 应该比 --bg 亮");
  assert.ok(sum(vars["--panel-2"]) > sum(vars["--panel"]), "--panel-2 应该比 --panel 更亮");
  assert.ok(sum(vars["--line"]) > sum(vars["--panel-2"]), "--line 应该是最亮的那层");
});

test("亮色主题:面板比背景暗,color-scheme 为 light", () => {
  const { vars, colorScheme } = deriveUiColors(SOLARIZED_LIGHT);
  assert.equal(colorScheme, "light");
  assert.ok(sum(vars["--panel"]) < sum(vars["--bg"]), "浅底上面板应该更暗");
  assert.ok(sum(vars["--panel-2"]) < sum(vars["--panel"]));
  assert.ok(sum(vars["--line"]) < sum(vars["--panel-2"]));
});

test("--dim 落在前景与背景之间(两种主题都是)", () => {
  for (const theme of [TOKYO, SOLARIZED_LIGHT]) {
    const { vars } = deriveUiColors(theme);
    const [lo, hi] = [sum(vars["--bg"]), sum(vars["--text"])].sort((a, b) => a - b);
    assert.ok(sum(vars["--dim"]) > lo && sum(vars["--dim"]) < hi, `--dim 越界: ${vars["--dim"]}`);
  }
});

test("强调色/危险色取自主题", () => {
  assert.equal(deriveUiColors(TOKYO).vars["--accent"], "#7aa2f7");
  assert.equal(deriveUiColors(TOKYO).vars["--danger"], "#f7768e");
  // 只有亮色变体时用亮色变体
  const only = deriveUiColors({ ...TOKYO, blue: undefined, brightBlue: "#89b4fa" });
  assert.equal(only.vars["--accent"], "#89b4fa");
  // 一个蓝色都没有时回退到前景色,而不是留空
  const none = deriveUiColors({ background: "#000000", foreground: "#ffffff" });
  assert.equal(none.vars["--accent"], "#ffffff");
});

test("常规色在背景上太暗时升级到亮色变体", () => {
  // 取自目录里的 iTerm2 Default:blue #2225c4 配纯黑对比度只有 2.13,不能用
  const iterm2 = {
    background: "#000000",
    foreground: "#ffffff",
    blue: "#2225c4",
    brightBlue: "#6871ff",
    red: "#c91b00",
    brightRed: "#ff6e67",
  };
  const { vars } = deriveUiColors(iterm2);
  assert.equal(vars["--accent"], "#6871ff", "过暗的 blue(2.13)应该让位给 brightBlue");
  // 同一个主题的 red 是 3.64,已经过线,不该被顶替 —— 升级是逐色判断的,不是整体开关
  assert.equal(vars["--danger"], "#c91b00");
});

test("常规色够用时不动它(尊重主题作者的选择)", () => {
  // TokyoNight 的 blue 对背景对比度 6.79,远够用,不该被 brightBlue 顶替
  const withBright = { ...TOKYO, brightBlue: "#ffffff" };
  assert.equal(deriveUiColors(withBright).vars["--accent"], "#7aa2f7");
});

test("常规色和亮色都不够时取两者中较好的那个", () => {
  const bad = { background: "#000000", foreground: "#ffffff", blue: "#050510", brightBlue: "#202060" };
  assert.equal(deriveUiColors(bad).vars["--accent"], "#202060");
});

test("每个内置主题派生出的强调色都达到 UI 最低对比度", async () => {
  const { BUILTIN_THEMES } = await import("./.builtin.build.mjs");
  const lin = (c) => (c / 255 <= 0.03928 ? c / 255 / 12.92 : Math.pow((c / 255 + 0.055) / 1.055, 2.4));
  const lum = (h) => {
    const n = parseInt(h.slice(1), 16);
    return 0.2126 * lin((n >> 16) & 255) + 0.7152 * lin((n >> 8) & 255) + 0.0722 * lin(n & 255);
  };
  const cr = (a, b) => {
    const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  };
  for (const [name, theme] of Object.entries(BUILTIN_THEMES)) {
    const { vars } = deriveUiColors(theme);
    for (const key of ["--accent", "--danger", "--text"]) {
      const ratio = cr(vars[key], vars["--bg"]);
      assert.ok(ratio >= 3, `${name} 的 ${key} (${vars[key]}) 对比度只有 ${ratio.toFixed(2)}`);
    }
  }
});

test("缺前景或背景时返回 null(维持 style.css 原样)", () => {
  assert.equal(deriveUiColors({ foreground: "#ffffff" }), null);
  assert.equal(deriveUiColors({ background: "#000000" }), null);
  assert.equal(deriveUiColors({}), null);
});

test("短写十六进制与带 alpha 的形式都能解析", () => {
  const short = deriveUiColors({ background: "#000", foreground: "#fff" });
  assert.equal(short.vars["--bg"], "#000000");
  assert.equal(short.vars["--text"], "#ffffff");
  const alpha = deriveUiColors({ background: "#1a1b26ff", foreground: "#c0caf5cc" });
  assert.equal(alpha.vars["--bg"], "#1a1b26");
});

test("isLightTheme 按背景亮度判断", () => {
  assert.equal(isLightTheme(TOKYO), false);
  assert.equal(isLightTheme(SOLARIZED_LIGHT), true);
  assert.equal(isLightTheme({}), false);
});
