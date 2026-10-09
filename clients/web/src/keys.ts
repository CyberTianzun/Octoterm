/**
 * Windows 的 ConPTY 从字节重建 INPUT_RECORD。xterm 的传统 VT 编码会丢掉
 * Ctrl+J 的 J、Enter 的修饰键,还会把 Alt+方向键改成 Ctrl+方向键。
 * 在 xterm 编码之前统一转成 win32-input-mode:CSI Vk;Sc;Uc;Kd;Cs;Rc _。
 * 输入法、粘贴和终端应答仍走 xterm.onData。
 */
export type ServerOs = string;

interface KeyLike {
  type: string;
  key: string;
  code: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  keyCode?: number;
  isComposing?: boolean;
  getModifierState?: (key: string) => boolean;
}

interface WinKey { vk: number; scan: number; enhanced?: boolean }
interface PressedKey extends WinKey { char: number }

// Set 1 扫描码取物理位置(code);扩展键以 ENHANCED_KEY 标记,不把 E0 塞进 Sc。
const codes: Record<string, WinKey> = {
  Escape: { vk: 27, scan: 1 }, Backspace: { vk: 8, scan: 14 }, Tab: { vk: 9, scan: 15 },
  Enter: { vk: 13, scan: 28 }, Space: { vk: 32, scan: 57 },
  Minus: { vk: 189, scan: 12 }, Equal: { vk: 187, scan: 13 },
  BracketLeft: { vk: 219, scan: 26 }, BracketRight: { vk: 221, scan: 27 },
  Semicolon: { vk: 186, scan: 39 }, Quote: { vk: 222, scan: 40 },
  Backquote: { vk: 192, scan: 41 }, Backslash: { vk: 220, scan: 43 },
  Comma: { vk: 188, scan: 51 }, Period: { vk: 190, scan: 52 }, Slash: { vk: 191, scan: 53 },
  IntlBackslash: { vk: 226, scan: 86 },
  ShiftLeft: { vk: 16, scan: 42 }, ShiftRight: { vk: 16, scan: 54 },
  ControlLeft: { vk: 17, scan: 29 }, ControlRight: { vk: 17, scan: 29, enhanced: true },
  AltLeft: { vk: 18, scan: 56 }, AltRight: { vk: 18, scan: 56, enhanced: true },
  CapsLock: { vk: 20, scan: 58 }, NumLock: { vk: 144, scan: 69, enhanced: true },
  ScrollLock: { vk: 145, scan: 70 }, Pause: { vk: 19, scan: 69 },
  PrintScreen: { vk: 44, scan: 55, enhanced: true }, ContextMenu: { vk: 93, scan: 93, enhanced: true },
  Insert: { vk: 45, scan: 82, enhanced: true }, Delete: { vk: 46, scan: 83, enhanced: true },
  Home: { vk: 36, scan: 71, enhanced: true }, End: { vk: 35, scan: 79, enhanced: true },
  PageUp: { vk: 33, scan: 73, enhanced: true }, PageDown: { vk: 34, scan: 81, enhanced: true },
  ArrowUp: { vk: 38, scan: 72, enhanced: true }, ArrowDown: { vk: 40, scan: 80, enhanced: true },
  ArrowLeft: { vk: 37, scan: 75, enhanced: true }, ArrowRight: { vk: 39, scan: 77, enhanced: true },
  NumpadEnter: { vk: 13, scan: 28, enhanced: true },
  NumpadMultiply: { vk: 106, scan: 55 }, NumpadAdd: { vk: 107, scan: 78 },
  NumpadSubtract: { vk: 109, scan: 74 }, NumpadDecimal: { vk: 110, scan: 83 },
  NumpadDivide: { vk: 111, scan: 53, enhanced: true },
};
for (const [letters, scan] of [["QWERTYUIOP", 16], ["ASDFGHJKL", 30], ["ZXCVBNM", 44]] as const) {
  [...letters].forEach((letter, i) => { codes[`Key${letter}`] = { vk: letter.charCodeAt(0), scan: scan + i }; });
}
for (let i = 0; i < 10; i++) codes[`Digit${i}`] = { vk: 48 + i, scan: i === 0 ? 11 : i + 1 };
for (let i = 1; i <= 24; i++) codes[`F${i}`] = { vk: 111 + i, scan: i <= 10 ? 58 + i : i <= 12 ? 76 + i : 0 };
[82, 79, 80, 81, 75, 76, 77, 71, 72, 73].forEach((scan, i) => { codes[`Numpad${i}`] = { vk: 96 + i, scan }; });

function packet(key: PressedKey, down: boolean, state: number): string {
  return `\x1b[${key.vk};${key.scan};${key.char};${down ? 1 : 0};${state};1_`;
}

function resolveKey(ev: KeyLike): WinKey | null {
  const physical = codes[ev.code];
  if (!physical) return null;
  const named = codes[ev.key];
  if (ev.code.startsWith("Numpad") && named) {
    // NumLock 关时仍是小键盘扫描码,虚拟键则是 Home/方向键/Delete 等。
    return { ...physical, vk: named.vk };
  }
  // 字母虚拟键按布局实际产出的字母取值,扫描码保持物理位置。
  if (/^[a-z]$/i.test(ev.key)) return { ...physical, vk: ev.key.toUpperCase().charCodeAt(0) };
  return physical;
}

function character(ev: KeyLike, key: WinKey): number {
  if (key.vk === 13) return ev.ctrlKey ? 10 : 13;
  if (key.vk === 9) return 9;
  if (key.vk === 8) return ev.ctrlKey ? 127 : 8;
  if (key.vk === 27) return 27;
  if (ev.key.length !== 1) return 0;
  const ch = ev.key.charCodeAt(0);
  if (!ev.ctrlKey) return ch;
  if (ev.key === " " || ev.key === "@") return 0;
  if (ev.key === "/" || ev.key === "_") return 31;
  if (ev.key === "?") return 127;
  if (/^[2-8]$/.test(ev.key)) return [0, 27, 28, 29, 30, 31, 127][ch - 50];
  if ((ch >= 64 && ch <= 95) || (ch >= 97 && ch <= 122)) return ch & 31;
  return ch;
}

/** 这些事件需要浏览器默认动作;调用方应跳过 xterm,但不要 preventDefault。 */
export function browserShortcut(ev: KeyLike, hasSelection: boolean): boolean {
  if (ev.metaKey || ev.key === "Meta") return true;
  if (ev.ctrlKey && !ev.altKey) {
    const key = ev.key.toLowerCase();
    if (key === "v" || (key === "c" && (ev.shiftKey || hasSelection)) || ev.key === "Insert") return true;
    // xterm 原来让这些 Ctrl+Shift 浏览器命令穿过,适配层也不截获。
    if (ev.shiftKey && ["i", "j", "r", "n", "t", "w", "p"].includes(key)) return true;
  }
  return ev.shiftKey && !ev.ctrlKey && !ev.altKey && ev.key === "Insert";
}

/** 每个终端一份状态,避免丢焦点后修饰键保持按下、或孤立的 keyup 进 PTY。 */
export class WindowsKeyboardAdapter {
  private pressed = new Map<string, PressedKey>();
  private composing = false;
  private altGraph = false;

  /** macOS 的 Option 默认用于布局文本,只有功能键或 macOptionIsMeta 才作为 Alt。 */
  constructor(private readonly optionAsText = false) {}

  setComposing(value: boolean): void { this.composing = value; }

  /** null = 交给 xterm;字符串 = 原样发 PTY 并取消事件;空串 = 只取消事件。 */
  encode(ev: KeyLike, serverOs: ServerOs, hasSelection = false): string | null {
    if (serverOs !== "windows") return null;
    // 部分 Windows 浏览器不报告 getModifierState("AltGraph"),但会报告右 Alt+Ctrl。
    if (ev.code === "AltRight") {
      if (ev.type === "keydown") this.altGraph = ev.key === "AltGraph" || ev.ctrlKey;
      if (ev.type === "keyup") this.altGraph = false;
    }
    const previous = this.pressed.get(ev.code);
    if (ev.type === "keyup") {
      if (!previous) return null;
      this.pressed.delete(ev.code);
      return packet(previous, false, this.controlState(ev, previous));
    }
    if (ev.type === "keypress") return previous ? "" : null;
    if (ev.type !== "keydown") return null;
    if (this.composing || ev.isComposing || ev.keyCode === 229 ||
        ["Dead", "Process", "Unidentified", "AltGraph"].includes(ev.key)) return null;
    if (browserShortcut(ev, hasSelection)) return null;
    // AltGr 的布局文本由浏览器提交,不把它当 Ctrl+Alt 快捷键。
    if ((this.altGraph || ev.getModifierState?.("AltGraph")) && ev.key.length === 1) return null;
    if (this.optionAsText && ev.altKey && !ev.ctrlKey && ev.key.length === 1) return null;
    // 非 ASCII 文本(包括代理对)交给 xterm,保留它的输入法/Unicode 处理。
    if (/[^\x00-\x7f]/.test(ev.key)) return null;
    const key = resolveKey(ev);
    if (!key) return null;
    const pressed = { ...key, char: character(ev, key) };
    this.pressed.set(ev.code, pressed);
    // 自动重复各发一次按下,Rc=1;只有真实 keyup 才发送松开。
    return packet(pressed, true, this.controlState(ev, pressed));
  }

  private controlState(ev: KeyLike, key: WinKey): number {
    let state = key.enhanced ? 0x100 : 0;
    if (ev.shiftKey) state |= 0x10;
    if (ev.ctrlKey) {
      const right = this.pressed.has("ControlRight");
      if (right) state |= 0x4;
      if (this.pressed.has("ControlLeft") || !right) state |= 0x8;
    }
    if (ev.altKey) {
      const right = this.pressed.has("AltRight");
      if (right) state |= 0x1;
      if (this.pressed.has("AltLeft") || !right) state |= 0x2;
    }
    if (ev.getModifierState?.("CapsLock")) state |= 0x80;
    if (ev.getModifierState?.("NumLock")) state |= 0x20;
    if (ev.getModifierState?.("ScrollLock")) state |= 0x40;
    return state;
  }

  /** blur/detach 时释放已转发的键。断线时只 reset,不向断开的 socket 发包。 */
  releaseAll(): string {
    const release = [...this.pressed.values()].map(key => packet(key, false, key.enhanced ? 0x100 : 0)).join("");
    this.reset();
    return release;
  }

  reset(): void { this.pressed.clear(); this.composing = false; this.altGraph = false; }
}
