import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { execSync } from "node:child_process";

// 同 config.test.mjs:node 跑不了 .ts,用 esbuild 即时转译。
const here = dirname(fileURLToPath(import.meta.url));
execSync("npx esbuild src/i18n.ts --bundle --format=esm --outfile=test/.i18n.build.mjs", {
  cwd: join(here, ".."),
});
const { LOCALES, LOCALE_NAMES, LOCALE_TAGS, catalog, getLocale, localeTag, resolveLocale, setLocale, subscribe, t } =
  await import("./.i18n.build.mjs");

const placeholders = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

/* ---------- 词条表本身 ---------- */

// tsc 已经拦得住「少一条键」,但拦不住多一条、也拦不住占位符对不上 ——
// 后者会让某个语言下的消息永远缺一半信息,而且只在运行到那一行时才看得出来。
test("每个语言的词条键完全一致", () => {
  const ref = Object.keys(catalog("zh-CN")).sort();
  for (const l of LOCALES) {
    assert.deepEqual(Object.keys(catalog(l)).sort(), ref, l);
  }
});

test("同一条消息在各语言里的占位符集合一致", () => {
  const zh = catalog("zh-CN");
  for (const l of LOCALES) {
    const c = catalog(l);
    for (const key of Object.keys(zh)) {
      assert.deepEqual(placeholders(c[key]), placeholders(zh[key]), `${l} / ${key}`);
    }
  }
});

test("没有空词条", () => {
  for (const l of LOCALES) {
    for (const [key, value] of Object.entries(catalog(l))) {
      assert.ok(value.trim().length > 0, `${l} / ${key}`);
    }
  }
});

test("每个语言都有母语显示名和 BCP 47 标签", () => {
  for (const l of LOCALES) {
    assert.ok(LOCALE_NAMES[l], l);
    assert.ok(LOCALE_TAGS[l], l);
  }
});

/* ---------- index.html 的静态文案 ---------- */

// data-i18n 里写错一个 key,页面上就直接显示那串 key 本身 —— 这是唯一一处
// 类型系统看不见的词条引用(HTML 属性里的字符串),所以在这里兜住。
test("index.html 引用的词条键都存在", async () => {
  const { readFileSync } = await import("node:fs");
  const html = readFileSync(join(here, "../src/index.html"), "utf-8");
  const keys = [...html.matchAll(/data-i18n(?:-title)?="([^"]+)"/g)].map((m) => m[1]);
  assert.ok(keys.length > 0, "index.html 里应该有 data-i18n 标记");
  const known = catalog("zh-CN");
  for (const k of keys) {
    assert.ok(Object.prototype.hasOwnProperty.call(known, k), `未知词条 ${k}`);
  }
});

/* ---------- 语言解析 ---------- */

test("显式偏好压过浏览器语言", () => {
  assert.equal(resolveLocale("en", ["zh-CN"]), "en");
  assert.equal(resolveLocale("zh-CN", ["en-US"]), "zh-CN");
});

test("auto 时按浏览器语言依次匹配,只看主子标签", () => {
  assert.equal(resolveLocale("auto", ["zh-CN", "en"]), "zh-CN");
  assert.equal(resolveLocale("auto", ["en-GB"]), "en");
  // 繁体暂时归到简体:一份看得懂的中文界面胜过一份英文界面
  assert.equal(resolveLocale("auto", ["zh-Hant-TW"]), "zh-CN");
  // 前面几个都不支持时继续往后找
  assert.equal(resolveLocale("auto", ["fr", "de", "zh"]), "zh-CN");
});

test("匹配不到任何支持的语言时回落 en", () => {
  assert.equal(resolveLocale("auto", []), "en");
  assert.equal(resolveLocale("auto", ["fr-FR", "de"]), "en");
  // 存进配置的偏好也可能是被手改坏的,同样走回落而不是抛
  assert.equal(resolveLocale("klingon", ["fr"]), "en");
});

/* ---------- t() ---------- */

test("占位符替换;没给到的原样留下", () => {
  setLocale("en");
  assert.equal(t("settings.io.ok", { source: "Import" }), "Import succeeded");
  assert.match(t("settings.io.ok"), /\{source\}/);
});

test("切语言后 t() 立刻返回新语言的文案", () => {
  setLocale("zh-CN");
  assert.equal(getLocale(), "zh-CN");
  assert.equal(localeTag(), "zh-CN");
  const zh = t("app.settings");
  setLocale("en");
  assert.equal(t("app.settings"), "Settings");
  assert.notEqual(zh, t("app.settings"));
});

test("订阅者只在语言真的变了时被通知", () => {
  setLocale("en");
  let calls = 0;
  const off = subscribe(() => calls++);
  setLocale("en"); // 同值:空操作
  assert.equal(calls, 0);
  setLocale("zh-CN");
  assert.equal(calls, 1);
  off();
  setLocale("en");
  assert.equal(calls, 1);
});
