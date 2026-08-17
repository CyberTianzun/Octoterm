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
    const preview = new Terminal({ cols: s.cols, rows: s.rows, disableStdin: true, fontSize: 5 });
    preview.open(row.querySelector(".preview") as HTMLElement);
    previews.set(s.id, preview);
    client.send({ type: "preview", id: s.id });
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
    term?.dispose();
    term = null;
  }
  attachedId = id;
  $("workspace-empty").hidden = true;
  $("terminal-wrap").hidden = false;
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
  $("workspace-empty").hidden = false;
  client.send({ type: "list-sessions" });
}

function refit() {
  if (!term || !fit) return;
  fit.fit();
  client.resize(TERM_CHANNEL, term.cols, term.rows);
}

window.addEventListener("resize", refit);
window.visualViewport?.addEventListener("resize", refit);
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

client.connect(wsUrl(), token());
