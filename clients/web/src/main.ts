import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { OctoClient } from "./client";

const $ = (id: string) => document.getElementById(id)!;
const client = new OctoClient();
const TERM_CHANNEL = 1;

let sessions: any[] = [];
let attachedId: number | null = null;
let term: Terminal | null = null;
let fit: FitAddon | null = null;
const previews = new Map<number, Terminal>();

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

function sessionActions(el: HTMLElement, s: any) {
  el.addEventListener("click", (ev) => {
    const act = (ev.target as HTMLElement).dataset?.act;
    if (act === "kill") {
      ev.stopPropagation();
      client.send({ type: "kill-session", id: s.id });
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
}

/* ---------- sidebar:始终渲染 ---------- */
function renderSidebar() {
  const nav = $("session-nav");
  nav.innerHTML = "";
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
      <div class="sacts">
        <button data-act="rename" title="改名">✎</button>
        <button data-act="kill" title="结束会话">✕</button>
      </div>`;
    row.querySelector(".sname")!.textContent = s.name;
    row.querySelector(".smeta")!.textContent = `${s.cols}×${s.rows} · ${fmtTime(s.created_at)}`;
    sessionActions(row, s);
    nav.appendChild(row);
  }
}

/* ---------- 工作区空态:预览卡片仪表盘 ---------- */
function renderDashboard() {
  const dash = $("dashboard");
  dash.innerHTML = "";
  previews.forEach((t) => t.dispose());
  previews.clear();
  if (sessions.length === 0) {
    dash.innerHTML = `<div class="empty">没有运行中的会话</div>`;
    return;
  }
  for (const s of sessions) {
    const card = document.createElement("div");
    card.className = "card";
    card.innerHTML = `
      <div class="preview"></div>
      <div class="row"><span class="name"></span>
        <button data-act="rename">改名</button>
        <button data-act="kill">结束</button></div>
      <div class="meta"></div>`;
    card.querySelector(".name")!.textContent = s.name;
    card.querySelector(".meta")!.textContent = `${s.cols}×${s.rows} · ${fmtTime(s.created_at)}`;
    const preview = new Terminal({ cols: s.cols, rows: s.rows, disableStdin: true, fontSize: 6 });
    preview.open(card.querySelector(".preview") as HTMLElement);
    previews.set(s.id, preview);
    client.send({ type: "preview", id: s.id });
    sessionActions(card, s);
    dash.appendChild(card);
  }
}

function setDrawer(open: boolean) {
  document.body.classList.toggle("sidebar-open", open);
  $("scrim").hidden = !open;
}

function openTerminal(id: number) {
  setDrawer(false);
  if (attachedId === id) {
    term?.focus();
    return;
  }
  if (attachedId !== null) {
    client.detach(TERM_CHANNEL);
    term?.dispose();
    term = null;
  }
  attachedId = id;
  // 进入终端时预览不再可见,释放实例
  previews.forEach((t) => t.dispose());
  previews.clear();
  $("dashboard").hidden = true;
  $("terminal-wrap").hidden = false;
  $("detach").hidden = false;
  const s = sessions.find((x) => x.id === id);
  $("work-title").textContent = s ? s.name : `session ${id}`;
  term = new Terminal({ allowProposedApi: true });
  fit = new FitAddon();
  term.loadAddon(fit);
  term.open($("terminal"));
  fit.fit();
  term.onData((d) => client.sendInput(TERM_CHANNEL, new TextEncoder().encode(d)));
  client.attach(id, TERM_CHANNEL, term.cols, term.rows);
  term.focus();
  renderSidebar();
}

function closeTerminal() {
  if (attachedId !== null) client.detach(TERM_CHANNEL);
  attachedId = null;
  term?.dispose();
  term = null;
  $("terminal-wrap").hidden = true;
  $("detach").hidden = true;
  $("dashboard").hidden = false;
  $("work-title").textContent = "会话";
  client.send({ type: "list-sessions" });
  renderSidebar();
}

function refit() {
  if (!term || !fit) return;
  fit.fit();
  client.resize(TERM_CHANNEL, term.cols, term.rows);
}

window.addEventListener("resize", refit);
window.visualViewport?.addEventListener("resize", refit);
$("detach").addEventListener("click", closeTerminal);
$("menu").addEventListener("click", () => setDrawer(true));
$("scrim").addEventListener("click", () => setDrawer(false));
$("new-session").addEventListener("click", () =>
  client.send({ type: "new-session", name: prompt("Session name (optional):") || null, command: null }),
);

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
      if (attachedId === null) {
        renderDashboard();
      } else {
        const s = sessions.find((x) => x.id === attachedId);
        if (s) $("work-title").textContent = s.name;
      }
      break;
    case "session-event":
      client.send({ type: "list-sessions" });
      break;
    case "preview-data": {
      const p = previews.get(msg.id);
      if (p) p.write(Uint8Array.from(atob(msg.data), (c) => c.charCodeAt(0)));
      break;
    }
    case "resync-begin":
      term?.reset();
      break;
    case "session-exited":
      if (msg.channel === TERM_CHANNEL) closeTerminal();
      break;
    case "error":
      console.warn("octoterm error:", msg.message);
      // 重连打到一个已经不存在的会话上(channel 对应的 attach 失败):
      // 该终端已没有意义,退回仪表盘,而不是停留在一个死连接上。
      if (msg.channel === TERM_CHANNEL && attachedId !== null) closeTerminal();
      break;
  }
};

client.connect(wsUrl(), token());
