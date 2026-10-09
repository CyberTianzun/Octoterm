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
const { browserShortcut, WindowsKeyboardAdapter } = await import("./.keys.build.mjs");
const key = (over = {}) => ({
  type: "keydown", key: "Enter", code: "Enter", shiftKey: false, ctrlKey: false, altKey: false, metaKey: false,
  ...over,
});
const decode = seq => {
  assert.match(seq, /^\x1b\[(?:\d+;){5}\d+_$/);
  const [vk, scan, char, down, state, repeat] = seq.slice(2, -1).split(";").map(Number);
  return {vk, scan, char, down, state, repeat};
};
const encode = (ev, adapter = new WindowsKeyboardAdapter()) => adapter.encode(key(ev), "windows");

test("Shift+Enter 的真实按下和松开分别转发,不在按下时提前松开", () => {
  const adapter = new WindowsKeyboardAdapter();
  assert.deepEqual(decode(encode({shiftKey:true}, adapter)), {vk:13,scan:28,char:13,down:1,state:16,repeat:1});
  assert.deepEqual(decode(encode({type:"keyup",shiftKey:true}, adapter)), {vk:13,scan:28,char:13,down:0,state:16,repeat:1});
  assert.equal(adapter.releaseAll(), "");
});

test("Ctrl+J/M/I 的字符仍是 LF/CR/Tab,虚拟键仍是字母而不是 Enter/Tab", () => {
  for (const [letter, vk, char] of [["j",74,10],["m",77,13],["i",73,9]]) {
    const record = decode(encode({key:letter,code:`Key${letter.toUpperCase()}`,ctrlKey:true}));
    assert.equal(record.vk, vk);
    assert.equal(record.char, char);
    assert.equal(record.state, 8);
  }
});

test("常用功能键的八种 Ctrl/Shift/Alt 组合完整保留,不走 Alt-arrow 重映射或本地滚屏", () => {
  for (const [name, vk, enhanced] of [["Enter",13,false],["Tab",9,false],["Backspace",8,false],["ArrowUp",38,true],["ArrowLeft",37,true],["PageUp",33,true],["PageDown",34,true],["Home",36,true],["Delete",46,true],["F1",112,false],["F12",123,false]]) {
    for (let mask = 0; mask < 8; mask++) {
      const record = decode(encode({key:name,code:name,shiftKey:!!(mask&1),ctrlKey:!!(mask&2),altKey:!!(mask&4)}));
      assert.equal(record.vk, vk, `${name}/${mask}`);
      assert.equal(record.state, (enhanced?256:0) | ((mask&1)?16:0) | ((mask&2)?8:0) | ((mask&4)?2:0), `${name}/${mask}`);
    }
  }
});

test("重复按下不生成松开;keypress 不再生成第二份字符", () => {
  const adapter = new WindowsKeyboardAdapter();
  const ev = {key:"a",code:"KeyA"};
  assert.equal(decode(encode(ev, adapter)).down, 1);
  assert.equal(decode(encode({...ev,repeat:true}, adapter)).down, 1);
  assert.equal(encode({...ev,type:"keypress"}, adapter), "");
  assert.equal(decode(encode({...ev,type:"keyup"}, adapter)).down, 0);
  assert.equal(encode({...ev,type:"keyup"}, adapter), null);
});

test("keyup 即使修饰键先松开也释放原来的键,并使用当前修饰状态", () => {
  const adapter = new WindowsKeyboardAdapter();
  encode({key:"A",code:"KeyA",ctrlKey:true,shiftKey:true}, adapter);
  const up = decode(encode({type:"keyup",key:"a",code:"KeyA"}, adapter));
  assert.equal(up.vk, 65);
  assert.equal(up.char, 1);
  assert.equal(up.state, 0);
  assert.equal(up.down, 0);
});

test("左右 Ctrl/Alt 和锁定键状态独立保留", () => {
  const adapter = new WindowsKeyboardAdapter();
  encode({key:"Control",code:"ControlRight",ctrlKey:true}, adapter);
  encode({key:"Control",code:"ControlLeft",ctrlKey:true}, adapter);
  encode({key:"Alt",code:"AltRight",ctrlKey:true,altKey:true}, adapter);
  const ev = {key:"ArrowUp",code:"ArrowUp",ctrlKey:true,altKey:true,getModifierState:name => ["CapsLock","NumLock","ScrollLock"].includes(name)};
  assert.equal(decode(encode(ev, adapter)).state, 256|4|8|1|128|32|64);
  const up = decode(encode({type:"keyup",key:"Control",code:"ControlRight",ctrlKey:true,altKey:true}, adapter));
  assert.equal(up.state, 256|8|1);
});

test("NumLock 关的小键盘仍保留扫描码,不误标为扩展键", () => {
  assert.deepEqual(decode(encode({key:"End",code:"Numpad1"})), {vk:35,scan:79,char:0,down:1,state:0,repeat:1});
  assert.equal(decode(encode({key:"1",code:"Numpad1"})).vk, 97);
  assert.equal(decode(encode({key:"Enter",code:"NumpadEnter"})).state, 256);
});

test("字母虚拟键取实际布局字符,扫描码取物理位置;CapsLock 字符不被改写", () => {
  const record = decode(encode({key:"a",code:"KeyQ"}));
  assert.equal(record.vk, 65);
  assert.equal(record.scan, 16);
  assert.equal(record.char, 97);
  assert.equal(decode(encode({key:"A",code:"KeyA",getModifierState:n => n === "CapsLock"})).char, 65);
});

test("Ctrl+C 无选区时中断;有选区及复制/粘贴/Meta/浏览器命令交给浏览器", () => {
  const copy = key({key:"c",code:"KeyC",ctrlKey:true});
  assert.equal(decode(encode(copy)).char, 3);
  assert.equal(new WindowsKeyboardAdapter().encode(copy, "windows", true), null);
  assert.equal(browserShortcut(copy, true), true);
  for (const ev of [
    {key:"v",code:"KeyV",ctrlKey:true}, {key:"V",code:"KeyV",ctrlKey:true,shiftKey:true},
    {key:"C",code:"KeyC",ctrlKey:true,shiftKey:true}, {key:"Insert",code:"Insert",ctrlKey:true},
    {key:"Insert",code:"Insert",shiftKey:true}, {key:"c",code:"KeyC",metaKey:true},
    {key:"R",code:"KeyR",ctrlKey:true,shiftKey:true},
  ]) {
    assert.equal(encode(ev), null);
    assert.equal(browserShortcut(key(ev), false), true);
  }
});

test("输入法的候选确认 Enter、229、dead key、AltGr 和 Unicode 文本不被截获", () => {
  const adapter = new WindowsKeyboardAdapter();
  adapter.setComposing(true);
  assert.equal(encode({shiftKey:true}, adapter), null);
  adapter.setComposing(false);
  assert.notEqual(encode({shiftKey:true}, adapter), null);
  for (const ev of [
    {isComposing:true}, {keyCode:229}, {key:"Dead",code:"Quote"}, {key:"Process"},
    {key:"Unidentified",code:"KeyA"}, {key:"AltGraph",code:"AltRight"},
    {key:"@",code:"KeyQ",ctrlKey:true,altKey:true,getModifierState:n => n === "AltGraph"},
    {key:"é",code:"KeyE"}, {key:"中",code:"KeyA"}, {key:"😀",code:"KeyA"},
    {key:"a",code:""}, {code:"FutureKey"},
  ]) assert.equal(encode(ev), null, JSON.stringify(ev));
});

test("丢焦点释放所有已经转发的键,然后不再转发孤立的 keyup", () => {
  const adapter = new WindowsKeyboardAdapter();
  encode({key:"Alt",code:"AltLeft",altKey:true}, adapter);
  encode({key:"ArrowUp",code:"ArrowUp",altKey:true}, adapter);
  const releases = adapter.releaseAll().match(/\x1b\[[\d;]+_/g).map(decode);
  assert.deepEqual(releases.map(r => [r.vk,r.down,r.state]), [[18,0,0],[38,0,256]]);
  assert.equal(encode({type:"keyup",key:"ArrowUp",code:"ArrowUp"}, adapter), null);
  assert.equal(adapter.releaseAll(), "");
  encode({}, adapter);
  adapter.reset();
  assert.equal(adapter.releaseAll(), "");
});

test("没有 AltGraph 标志的右 Alt+Ctrl 仍作为 AltGr 文本,左 Alt+Ctrl 仍作为快捷键", () => {
  const adapter = new WindowsKeyboardAdapter();
  encode({key:"Alt",code:"AltRight",ctrlKey:true,altKey:true}, adapter);
  const text = {key:"@",code:"KeyQ",ctrlKey:true,altKey:true};
  assert.equal(encode(text, adapter), null);
  encode({type:"keyup",key:"Alt",code:"AltRight"}, adapter);
  assert.notEqual(encode(text, adapter), null);
  adapter.reset();
  encode({key:"AltGraph",code:"AltRight",ctrlKey:true,altKey:true}, adapter);
  assert.equal(encode(text, adapter), null);
});

test("macOS 客户端连 Windows 时保留 Option 布局文本,Option+功能键仍作为 Alt", () => {
  const adapter = new WindowsKeyboardAdapter(true);
  assert.equal(encode({key:"@",code:"Digit2",altKey:true}, adapter), null);
  assert.equal(decode(encode({key:"ArrowUp",code:"ArrowUp",altKey:true}, adapter)).state, 258);
  assert.equal(decode(encode({key:"j",code:"KeyJ",ctrlKey:true}, adapter)).vk, 74);
});

test("非 Windows 或未知服务端保留 xterm 行为", () => {
  const adapter = new WindowsKeyboardAdapter();
  for (const os of ["linux", "macos", ""]) {
    assert.equal(adapter.encode(key({shiftKey:true}), os), null);
    assert.equal(adapter.encode(key({type:"keyup",shiftKey:true}), os), null);
  }
  assert.equal(adapter.releaseAll(), "");
});
