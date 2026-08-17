import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { execSync } from "node:child_process";

// 同 protocol.test.mjs:node 跑不了 .ts,用 esbuild 即时转译。config.ts 对
// @xterm/xterm 只有 `import type`,esbuild 会整个剥掉,所以这里不会把浏览器
// 端的 xterm 拽进 node 进程。
const here = dirname(fileURLToPath(import.meta.url));
for (const [src, out] of [
  ["src/config.ts", "test/.config.build.mjs"],
  // 默认主题跟随系统亮暗那几条要拿真的内置主题表和真的配色派生来验
  ["src/themes/builtin.ts", "test/.builtin.build.mjs"],
  ["src/appearance.ts", "test/.appearance.build.mjs"],
]) {
  execSync(`npx esbuild ${src} --bundle --format=esm --outfile=${out}`, { cwd: join(here, "..") });
}
const cfgmod = await import("./.config.build.mjs");
const {
  defaultConfig,
  sanitizeConfig,
  sanitizeTheme,
  importConfigJson,
  exportConfigJson,
  toTerminalOptions,
  toPreviewOptions,
  CONFIG_VERSION,
} = cfgmod;

const collect = () => {
  const warnings = [];
  return { warnings, warn: (m) => warnings.push(m) };
};

test("默认主题跟随系统亮暗", async () => {
  const { DEFAULT_THEMES, BUILTIN_THEMES } = await import("./.builtin.build.mjs");
  const dark = defaultConfig(true);
  const light = defaultConfig(false);
  assert.equal(dark.theme.name, DEFAULT_THEMES.dark);
  assert.equal(light.theme.name, DEFAULT_THEMES.light);
  assert.notEqual(dark.theme.name, light.theme.name);
  // 颜色是内联的完整副本,不是只存个名字
  assert.deepEqual(dark.theme.colors, BUILTIN_THEMES[DEFAULT_THEMES.dark]);
  assert.deepEqual(light.theme.colors, BUILTIN_THEMES[DEFAULT_THEMES.light]);
  // 且互不共享引用 —— 改一个不能污染内置表
  dark.theme.colors.background = "#deadbe";
  assert.notEqual(BUILTIN_THEMES[DEFAULT_THEMES.dark].background, "#deadbe");
});

test("两个默认主题都必须编进 bundle(首屏就要用,等不了 fetch)", async () => {
  const { DEFAULT_THEMES, BUILTIN_THEMES } = await import("./.builtin.build.mjs");
  for (const name of Object.values(DEFAULT_THEMES)) {
    assert.ok(name in BUILTIN_THEMES, `${name} 不在 BUILTIN_THEMES 里`);
  }
});

test("亮色默认主题确实是亮的,暗色确实是暗的", async () => {
  const { deriveUiColors } = await import("./.appearance.build.mjs");
  assert.equal(deriveUiColors(defaultConfig(true).theme.colors).colorScheme, "dark");
  assert.equal(deriveUiColors(defaultConfig(false).theme.colors).colorScheme, "light");
});

test("除主题外,亮暗两份默认配置完全一致", () => {
  const strip = (c) => ({ ...c, theme: null });
  assert.deepEqual(strip(defaultConfig(true)), strip(defaultConfig(false)));
});

test("默认配置是 sanitize 的不动点(亮暗两份都是)", () => {
  for (const prefersDark of [true, false]) {
    const d = defaultConfig(prefersDark);
    const w = collect();
    assert.deepEqual(sanitizeConfig(d, { warn: w.warn }), d);
    assert.deepEqual(w.warnings, [], `prefersDark=${prefersDark}`);
  }
});

test("导出再导入完全还原", () => {
  const d = defaultConfig();
  const { config, warnings } = importConfigJson(exportConfigJson(d));
  assert.deepEqual(config, d);
  assert.deepEqual(warnings, []);
});

test("导出的 JSON 自带全部颜色,不依赖主题目录", () => {
  const d = defaultConfig();
  const parsed = JSON.parse(exportConfigJson(d));
  assert.equal(typeof parsed.theme.name, "string");
  assert.equal(parsed.theme.colors.background, d.theme.colors.background);
  // resolveTheme 返回 undefined(模拟目录完全不可用)也照样还原
  const { config } = importConfigJson(exportConfigJson(d), () => undefined);
  assert.deepEqual(config.theme, d.theme);
});

test("非对象输入退回默认配置且不抛", () => {
  for (const bad of [null, 42, "x", [], undefined]) {
    assert.deepEqual(sanitizeConfig(bad), defaultConfig());
  }
});

test("数值越界被收敛并记 warning", () => {
  const w = collect();
  const c = sanitizeConfig(
    { font: { size: 9999, lineHeight: -3, letterSpacing: 500 }, terminal: { scrollback: 1e9 } },
    { warn: w.warn },
  );
  assert.equal(c.font.size, 48);
  assert.equal(c.font.lineHeight, 0.8);
  assert.equal(c.font.letterSpacing, 10);
  assert.equal(c.terminal.scrollback, 200_000);
  assert.equal(w.warnings.length, 4);
});

test("NaN / 非数字退回默认值", () => {
  const d = defaultConfig();
  const c = sanitizeConfig({ font: { size: "14", lineHeight: null } });
  assert.equal(c.font.size, d.font.size);
  assert.equal(c.font.lineHeight, d.font.lineHeight);
});

test("枚举字段只收白名单值", () => {
  const d = defaultConfig();
  const c = sanitizeConfig({ terminal: { cursorStyle: "rainbow", cursorInactiveStyle: "bar" } });
  assert.equal(c.terminal.cursorStyle, d.terminal.cursorStyle);
  assert.equal(c.terminal.cursorInactiveStyle, "bar");
});

test("只接受十六进制颜色 —— 颜色最终会进 CSS 自定义属性", () => {
  const w = collect();
  const theme = sanitizeTheme(
    {
      background: "#1a1b26",
      foreground: "#abc",
      cursor: "#11223344",
      red: "red",
      green: "rgb(0,255,0)",
      blue: "#fff; background:url(https://evil.example/x)",
      yellow: "var(--x)",
      white: 123,
    },
    w.warn,
  );
  assert.deepEqual(theme, { background: "#1a1b26", foreground: "#abc", cursor: "#11223344" });
  assert.equal(w.warnings.length, 5);
});

test("extendedAnsi 只保留合法项并截断", () => {
  const t = sanitizeTheme({ extendedAnsi: ["#000000", "nope", "#ffffff"] });
  assert.deepEqual(t.extendedAnsi, ["#000000", "#ffffff"]);
  const big = sanitizeTheme({ extendedAnsi: Array(500).fill("#010203") });
  assert.equal(big.extendedAnsi.length, 240);
  // 一个合法项都没有时不留下空数组
  assert.equal("extendedAnsi" in sanitizeTheme({ extendedAnsi: ["nope"] }), false);
});

test("font.family 剔除能撑破 CSS 声明的字符", () => {
  const w = collect();
  const c = sanitizeConfig({ font: { family: 'Fira Code; } body { display:none' } }, { warn: w.warn });
  // `;` `{` `}` `:` 全部剔除,只剩下没有语法意义的字面量
  assert.equal(c.font.family, "Fira Code  body  displaynone");
  assert.match(c.font.family, /^[A-Za-z0-9 ,._'"-]*$/);
  assert.ok(w.warnings.some((m) => m.includes("font.family")));
});

test("font.family 被清空后回退到默认字体栈", () => {
  const d = defaultConfig();
  assert.equal(sanitizeConfig({ font: { family: "()()" } }).font.family, d.font.family);
  assert.equal(sanitizeConfig({ font: { family: "   " } }).font.family, d.font.family);
});

test("font.family 超长被截断", () => {
  const c = sanitizeConfig({ font: { family: "a".repeat(500) } });
  assert.equal(c.font.family.length, 200);
});

test("只给主题名时从目录解析", () => {
  const custom = { background: "#101010", foreground: "#e0e0e0" };
  const c = sanitizeConfig({ theme: { name: "My Theme" } }, { resolveTheme: () => custom });
  assert.equal(c.theme.name, "My Theme");
  assert.deepEqual(c.theme.colors, custom);
});

test("主题名查不到时退回默认主题,但其余字段保留", () => {
  const w = collect();
  const c = sanitizeConfig(
    { theme: { name: "Nonexistent" }, font: { size: 20 }, ui: { webgl: false } },
    { resolveTheme: () => undefined, warn: w.warn },
  );
  assert.equal(c.theme.name, defaultConfig().theme.name);
  assert.equal(c.font.size, 20);
  assert.equal(c.ui.webgl, false);
  assert.ok(w.warnings.some((m) => m.includes("Nonexistent")));
});

test("内联的 colors 优先于目录", () => {
  const inline = { background: "#000000" };
  const c = sanitizeConfig(
    { theme: { name: "Dracula", colors: inline } },
    { resolveTheme: () => ({ background: "#ffffff" }) },
  );
  assert.equal(c.theme.colors.background, "#000000");
});

test("来自更高版本的配置能读,只是提示有字段被忽略", () => {
  const w = collect();
  const c = sanitizeConfig(
    { version: CONFIG_VERSION + 7, font: { size: 18 }, futureKnob: true },
    { warn: w.warn },
  );
  assert.equal(c.version, CONFIG_VERSION);
  assert.equal(c.font.size, 18);
  assert.equal("futureKnob" in c, false);
  assert.ok(w.warnings.some((m) => m.includes("更新的版本")));
});

test("坏 JSON 抛异常,坏字段不抛", () => {
  assert.throws(() => importConfigJson("{ not json"));
  const { config, warnings } = importConfigJson('{"font":{"size":"big"}}');
  assert.equal(config.font.size, defaultConfig().font.size);
  assert.ok(warnings.length > 0);
});

test("toTerminalOptions 映射到 xterm 选项", () => {
  const d = defaultConfig();
  const o = toTerminalOptions(d);
  assert.equal(o.fontSize, d.font.size);
  assert.equal(o.fontFamily, d.font.family);
  assert.equal(o.scrollback, d.terminal.scrollback);
  assert.deepEqual(o.theme, d.theme.colors);
  // 不能悄悄带上构造期才该设的选项 —— term.options 是逐键合并的
  assert.equal("allowProposedApi" in o, false);
  assert.equal("cols" in o, false);
});

test("预览终端共享配色但钉死字号且禁用输入", () => {
  const d = defaultConfig();
  const o = toPreviewOptions(d);
  assert.deepEqual(o.theme, d.theme.colors);
  assert.equal(o.fontSize, 5);
  assert.equal(o.disableStdin, true);
  assert.equal(o.scrollback, 0);
});
