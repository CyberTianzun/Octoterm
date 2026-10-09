/**
 * xterm.js 按传统 VT 编码键盘:Shift+Enter 和 Enter 一样是 `\r`,Shift 丢了。
 * Unix 上的程序可以跟终端协商 kitty 键盘协议补回来;Windows 上中间隔着
 * ConPTY,它把输入字节还原成 INPUT_RECORD 交给程序(crossterm 读的就是这个),
 * `\r` 只能还原成没有修饰键的 Enter —— 程序永远看不到 Shift。
 *
 * ConPTY 认 win32-input-mode 序列 `CSI Vk;Sc;Uc;Kd;Cs;Rc _`,能原样带上修饰键,
 * Windows Terminal 就是靠它让 Shift+Enter 在 Codex 里换行的。这里只为 xterm
 * 编不出来的组合发这种序列,其余按键仍交给 xterm。
 */

/** 服务端在 hello-ok 里报的 `std::env::consts::OS`;空串表示旧服务端、未知。 */
export type ServerOs = string;

interface KeyLike {
  type: string;
  key: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}

/** dwControlKeyState 里的 SHIFT_PRESSED。 */
const SHIFT_PRESSED = 0x10;
const VK_RETURN = 0x0d;
const SCAN_RETURN = 0x1c;

function win32Key(vk: number, scan: number, char: number, down: boolean, state: number): string {
  return `\x1b[${vk};${scan};${char};${down ? 1 : 0};${state};1_`;
}

/**
 * 这个按键要不要绕过 xterm 自己编码。
 * - 返回 `null`:不管,交给 xterm。
 * - 返回字符串(可能为空):由调用方原样发给 PTY,并阻止 xterm 再处理。
 *   keyup 返回空串 —— 按下时已经把"按下+松开"一起发了,松开只需吞掉。
 */
export function encodeKey(ev: KeyLike, serverOs: ServerOs): string | null {
  if (serverOs !== "windows") return null;
  if (ev.key !== "Enter" || !ev.shiftKey || ev.ctrlKey || ev.altKey || ev.metaKey) return null;
  if (ev.type !== "keydown") return "";
  return (
    win32Key(VK_RETURN, SCAN_RETURN, 0x0d, true, SHIFT_PRESSED) +
    win32Key(VK_RETURN, SCAN_RETURN, 0x0d, false, SHIFT_PRESSED)
  );
}
