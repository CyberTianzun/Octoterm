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

function renderList() {
  const list = $("session-list");
  list.innerHTML = "";
  previews.forEach((t) => t.dispose());
  previews.clear();
  for (const s of sessions) {
    const card = document.createElement("div");
    card.className = "card";
    card.innerHTML = `
      <div class="preview"></div>
      <div class="row"><span class="name"></span>
        <button data-act="rename">Rename</button>
        <button data-act="kill">Kill</button>
        <button data-act="attach">Attach</button></div>`;
    const created = new Date(s.created_at * 1000).toLocaleString();
    card.querySelector(".name")!.textContent = `${s.name} · ${s.cols}×${s.rows} · ${created}`;
    const preview = new Terminal({ cols: s.cols, rows: s.rows, disableStdin: true, fontSize: 6 });
    preview.open(card.querySelector(".preview") as HTMLElement);
    previews.set(s.id, preview);
    client.send({ type: "preview", id: s.id });
    card.addEventListener("click", (ev) => {
      const act = (ev.target as HTMLElement).dataset?.act;
      if (act === "attach") openTerminal(s.id);
      if (act === "kill") client.send({ type: "kill-session", id: s.id });
      if (act === "rename") {
        const name = prompt("New name:", s.name);
        if (name) client.send({ type: "rename-session", id: s.id, name });
      }
    });
    list.appendChild(card);
  }
}

function openTerminal(id: number) {
  attachedId = id;
  $("session-list").hidden = true;
  $("terminal-page").hidden = false;
  $("back").hidden = false;
  term = new Terminal({ allowProposedApi: true });
  fit = new FitAddon();
  term.loadAddon(fit);
  term.open($("terminal"));
  fit.fit();
  term.onData((d) => client.sendInput(TERM_CHANNEL, new TextEncoder().encode(d)));
  client.attach(id, TERM_CHANNEL, term.cols, term.rows);
  term.focus();
}

function closeTerminal() {
  if (attachedId !== null) client.detach(TERM_CHANNEL);
  attachedId = null;
  term?.dispose();
  term = null;
  $("terminal-page").hidden = true;
  $("back").hidden = true;
  $("session-list").hidden = false;
  client.send({ type: "list-sessions" });
}

function refit() {
  if (!term || !fit) return;
  fit.fit();
  client.resize(TERM_CHANNEL, term.cols, term.rows);
}

window.addEventListener("resize", refit);
window.visualViewport?.addEventListener("resize", refit);
$("back").addEventListener("click", closeTerminal);
$("new-session").addEventListener("click", () =>
  client.send({ type: "new-session", name: prompt("Session name (optional):") || null, command: null }),
);

client.onOpen = () => {
  $("reconnect-banner").hidden = true;
  client.send({ type: "list-sessions" });
};
client.onReconnecting = () => {
  $("reconnect-banner").hidden = false;
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
      if (attachedId === null) renderList();
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
      break;
  }
};

client.connect(wsUrl(), token());
