import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { execSync } from "node:child_process";

// 同 launchers.test.mjs:node 跑不了 .ts,用 esbuild 即时转译。
const here = dirname(fileURLToPath(import.meta.url));
execSync("npx esbuild src/keys.ts --bundle --format=esm --outfile=test/.keys.build.mjs", {
  cwd: join(here, ".."),
});
const { encodeKey } = await import("./.keys.build.mjs");

const key = (over = {}) => ({
  type: "keydown", key: "Enter", shiftKey: true, ctrlKey: false, altKey: false, metaKey: false,
  ...over,
});

test("Windows 服务端:Shift+Enter 按下发 win32-input 的按下+松开", () => {
  assert.equal(
    encodeKey(key(), "windows"),
    "\x1b[13;28;13;1;16;1_\x1b[13;28;13;0;16;1_",
  );
});

test("Windows 服务端:Shift+Enter 松开吞掉、不再发", () => {
  assert.equal(encodeKey(key({ type: "keyup" }), "windows"), "");
});

test("非 Windows 或未知服务端:交给 xterm", () => {
  for (const os of ["linux", "macos", ""]) assert.equal(encodeKey(key(), os), null);
});

test("别的按键、别的修饰组合:交给 xterm", () => {
  assert.equal(encodeKey(key({ shiftKey: false }), "windows"), null);
  assert.equal(encodeKey(key({ ctrlKey: true }), "windows"), null);
  assert.equal(encodeKey(key({ altKey: true }), "windows"), null);
  assert.equal(encodeKey(key({ key: "a" }), "windows"), null);
});
