import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
// 版本被钉死(package.json 里没有 ^):addon 靠 `terminal._core` 上的私有字段干活,
// 跟 @xterm/xterm 是**同版本发布**的强耦合关系,但它没声明 peerDependencies,
// 装错了 npm 不会拦。0.18.0 配 xterm 5.5.0(同日发布);0.19.0 起对应 xterm 6。
// 错配的症状是 dispose 时抛 `Cannot read properties of undefined`。
// 升级 xterm 时这两个必须一起动。
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";
import { OctoClient } from "./client";
import { type OctoConfig, loadConfig, saveConfig, toTerminalOptions, toPreviewOptions } from "./config";
import { resolveTheme } from "./theme-catalog";
import { applyUiColors } from "./appearance";
import { mountSettings } from "./settings";
import { type Launcher, fetchLaunchers } from "./launchers";
import { mountNewSessionMenu } from "./new-session";

const $ = (id: string) => document.getElementById(id)!;
const client = new OctoClient();
const TERM_CHANNEL = 1;

let sessions: any[] = [];
let attachedId: number | null = null;
let term: Terminal | null = null;
let fit: FitAddon | null = null;
let webgl: WebglAddon | null = null;
const previews = new Map<number, Terminal>();

let config: OctoConfig = loadConfig(resolveTheme);

function token(): string {
  const fromHash = new URLSearchParams(location.hash.slice(1)).get("token");
  if (fromHash) localStorage.setItem("octoterm-token", fromHash);
  return localStorage.getItem("octoterm-token") ?? prompt("octoterm token:") ?? "";
}

function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/ws`;
}

function fmtTime(unixSecs: number): string {
  return new Date(unixSecs * 1000).toLocaleString();
}

/* ---------- 外观配置 ---------- */

/** 主题/字体改动的唯一落点:界面变量、主终端、所有预览、渲染器,一次全刷。 */
function applyConfig(next: OctoConfig) {
  const previewToggled = next.ui.sidebarPreview !== config.ui.sidebarPreview;
  config = next;
  saveConfig(config);
  applyUiColors(config);
  if (term) {
    term.options = toTerminalOptions(config);
    applyRenderer(term);
    // 字号/行高/字距一变,cell 尺寸就变了,能放下的 cols×rows 也跟着变。xterm
    // 要一帧才把新的 cell 尺寸量出来,这里等一帧再上报,否则量到的还是旧值。
    requestAnimationFrame(refit);
  }
  for (const p of previews.values()) p.options = toPreviewOptions(config);
  // 预览开关是唯一需要重建侧边栏 DOM 的改动:其余(主题/字形)对已存在的预览
  // 终端直接改 options 就够了。
  if (previewToggled) renderSidebar();
}

/** WebGL 渲染器。装载失败(没有 WebGL2 / 上下文丢失)静默回落到 DOM 渲染器 ——
 *  渲染器只影响性能,不该因为它连终端都开不出来。 */
function applyRenderer(t: Terminal) {
  if (!config.ui.webgl) {
    disposeWebgl();
    return;
  }
  if (webgl) return;
  try {
    const addon = new WebglAddon();
    addon.onContextLoss(() => {
      console.warn("octoterm: WebGL 上下文丢失,回落到 DOM 渲染器");
      if (webgl === addon) disposeWebgl();
      else addon.dispose();
    });
    t.loadAddon(addon);
    webgl = addon;
  } catch (err) {
    console.warn("octoterm: WebGL 渲染器不可用,使用 DOM 渲染器", err);
    webgl = null;
  }
}

const settings = mountSettings({
  get: () => config,
  set: (next) => applyConfig(next),
});

/* ---------- sidebar:选择/预览/改名/结束都在这里 ---------- */
function renderSidebar() {
  const nav = $("session-nav");
  nav.innerHTML = "";
  previews.forEach((t) => t.dispose());
  previews.clear();
  if (sessions.length === 0) {
    nav.innerHTML = `<div class="empty">还没有会话,点右上角 + 新建</div>`;
    return;
  }
  for (const s of sessions) {
    const row = document.createElement("div");
    row.className = "srow" + (s.id === attachedId ? " active" : "");
    row.innerHTML = `
      <div class="sname"></div>
      <div class="smeta"></div>
      <div class="preview"></div>
      <div class="sacts">
        <button data-act="rename" title="改名">✎</button>
        <button data-act="kill" title="结束会话">✕</button>
      </div>`;
    row.querySelector(".sname")!.textContent = s.name;
    row.querySelector(".smeta")!.textContent = `${s.cols}×${s.rows} · ${fmtTime(s.created_at)}`;
    const previewBox = row.querySelector(".preview") as HTMLElement;
    if (config.ui.sidebarPreview) {
      const preview = new Terminal({ cols: s.cols, rows: s.rows, ...toPreviewOptions(config) });
      preview.open(previewBox);
      previews.set(s.id, preview);
      client.send({ type: "preview", id: s.id });
    } else {
      previewBox.hidden = true;
    }
    row.addEventListener("click", (ev) => {
      const act = (ev.target as HTMLElement).dataset?.act;
      if (act === "kill") {
        ev.stopPropagation();
        client.send({ type: "kill-session", id: s.id });
        if (attachedId === s.id) closeTerminal();
        sessions = sessions.filter((x) => x.id !== s.id);
        renderSidebar();
        return;
      }
      if (act === "rename") {
        ev.stopPropagation();
        const name = prompt("New name:", s.name);
        if (name) client.send({ type: "rename-session", id: s.id, name });
        return;
      }
      openTerminal(s.id);
    });
    nav.appendChild(row);
  }
}

function setDrawer(open: boolean) {
  document.body.classList.toggle("sidebar-open", open);
  $("scrim").hidden = !open;
}

let errorTimer = 0;
function showError(message: string) {
  const el = $("error-banner");
  el.textContent = message;
  el.hidden = false;
  window.clearTimeout(errorTimer);
  errorTimer = window.setTimeout(() => {
    el.hidden = true;
  }, 8000);
}

/* ---------- 工作区:纯终端 ---------- */
function openTerminal(id: number) {
  setDrawer(false);
  if (attachedId === id) {
    term?.focus();
    return;
  }
  if (attachedId !== null) {
    client.detach(TERM_CHANNEL);
    disposeTerminal();
  }
  attachedId = id;
  $("workspace-empty").hidden = true;
  $("terminal-wrap").hidden = false;
  term = new Terminal({ allowProposedApi: true, ...toTerminalOptions(config) });
  fit = new FitAddon();
  term.loadAddon(fit);
  term.open($("terminal"));
  // WebGL addon 必须在 open() 之后装:它要拿 DOM 里的 canvas 上下文。
  applyRenderer(term);
  term.onData((d) => client.sendInput(TERM_CHANNEL, new TextEncoder().encode(d)));
  const want = proposeSize();
  client.attach(id, TERM_CHANNEL, want?.cols ?? term.cols, want?.rows ?? term.rows);
  term.focus();
  renderSidebar();
}

/**
 * 释放 WebGL addon。它的 dispose 会顺手把 DOM 渲染器装回去(addon 内部注册的
 * 恢复动作),所以关掉 WebGL 不需要我们自己重建渲染器。
 *
 * 包 try/catch 不是防御性洁癖:这条路径要动 WebGL 上下文和 xterm 的私有内部
 * (见文件顶部关于版本耦合的注释),失败是现实存在的。而它一旦抛出来,调用方的
 * `term.dispose()` 就再也执行不到,终端被吊在半空、attachedId 记账错位,会话
 * 切换会整个坏掉 —— 渲染器的清理失败不值得赔上这些。
 */
function disposeWebgl() {
  const addon = webgl;
  webgl = null; // 先清引用:即使 dispose 抛了,也不会留下一个已死的 addon
  try {
    addon?.dispose();
  } catch (err) {
    console.warn("octoterm: WebGL addon 释放失败,已忽略", err);
  }
}

/** addon 必须先于 Terminal 释放:0.18.0 的恢复路径会去碰 `_core._renderService`,
 *  Terminal 先没了就是对着一个拆掉的 render service 操作。 */
function disposeTerminal() {
  disposeWebgl();
  term?.dispose();
  term = null;
  fit = null;
}

function closeTerminal() {
  if (attachedId !== null) client.detach(TERM_CHANNEL);
  attachedId = null;
  disposeTerminal();
  $("terminal-wrap").hidden = true;
  $("workspace-empty").hidden = false;
  client.send({ type: "list-sessions" });
}

/** 量一下当前视口放得下多大。只作为"诉求"上报——多端 attach 同一个会话时 pty
 *  只有一个尺寸,权威值由服务端归并后经 resized 下发,所以这里不能直接 fit()。 */
function proposeSize(): { cols: number; rows: number } | null {
  if (!term || !fit) return null;
  const dims = fit.proposeDimensions();
  if (!dims || !Number.isFinite(dims.cols) || !Number.isFinite(dims.rows)) return null;
  if (dims.cols < 1 || dims.rows < 1) return null;
  return { cols: dims.cols, rows: dims.rows };
}

function refit() {
  const want = proposeSize();
  if (want) client.resize(TERM_CHANNEL, want.cols, want.rows);
}

window.addEventListener("resize", refit);
window.visualViewport?.addEventListener("resize", refit);
$("menu").addEventListener("click", () => setDrawer(true));
$("scrim").addEventListener("click", () => setDrawer(false));
$("open-settings").addEventListener("click", () => settings.open());

/**
 * 新建会话:选一个启动项就建,关掉菜单就是取消。
 *
 * 名字直接用启动项的名字(「zsh」「Prod SSH」),不再问一遍 —— 建完在侧边栏点 ✎
 * 就能改,为了改名先挡一个弹窗不划算。
 */
mountNewSessionMenu($("new-session"), {
  load: () => fetchLaunchers(authToken),
  pick: (l: Launcher) =>
    client.send({
      type: "new-session",
      name: l.name,
      // 空 argv 是兜底项的约定:让服务端用它自己的默认 shell
      command: l.command.length > 0 ? l.command : null,
      cwd: l.cwd,
    }),
});

client.onOpen = () => {
  $("reconnect-banner").hidden = true;
  $("conn-state").textContent = "已连接";
  client.send({ type: "list-sessions" });
};
client.onReconnecting = () => {
  $("reconnect-banner").textContent = "reconnecting…";
  $("reconnect-banner").hidden = false;
  $("conn-state").textContent = "重连中";
};
client.onFatal = (message) => {
  $("reconnect-banner").textContent = `连接失败: ${message} — 刷新页面重新输入 token`;
  $("reconnect-banner").hidden = false;
  $("conn-state").textContent = "已断开";
};
client.onChannelData = (channel, payload) => {
  if (channel === TERM_CHANNEL && term) {
    term.write(payload);
    client.noteData(channel, payload.length);
  }
};
client.onControl = (msg) => {
  switch (msg.type) {
    case "sessions":
      sessions = msg.sessions;
      renderSidebar();
      break;
    case "session-event":
      if (msg.event === "closed" && attachedId === msg.session?.id) {
        closeTerminal();
      }
      client.send({ type: "list-sessions" });
      break;
    case "preview-data": {
      const p = previews.get(msg.id);
      if (p) p.write(Uint8Array.from(atob(msg.data), (c) => c.charCodeAt(0)));
      break;
    }
    case "resized":
      // 服务端说了算:字节流是按这个尺寸换行的,自作主张按视口大小渲染会错位。
      // 视口比它大的部分留白(见 style.css 的居中),小则出现滚动条。
      if (msg.channel === TERM_CHANNEL) term?.resize(msg.cols, msg.rows);
      break;
    case "resync-begin":
      term?.reset();
      break;
    case "session-exited":
      if (msg.channel === TERM_CHANNEL) closeTerminal();
      break;
    case "error":
      console.warn("octoterm error:", msg.message);
      showError(msg.message);
      // 重连打到一个已经不存在的会话上(channel 对应的 attach 失败):
      // 该终端已没有意义,退回空态,而不是停留在一个死连接上。
      if (msg.channel === TERM_CHANNEL && attachedId !== null) closeTerminal();
      break;
  }
};

// 界面配色先于连接落地:否则首帧会闪一下 style.css 里那套写死的默认色
applyUiColors(config);
// 只取一次:token() 在 localStorage 里没有时会 prompt,取两次就要问两遍
const authToken = token();
client.connect(wsUrl(), authToken);
