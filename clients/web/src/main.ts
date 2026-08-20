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
import { type MsgKey, localeTag, navigatorLanguages, resolveLocale, setLocale, subscribe, t } from "./i18n";
import { mountNewSessionMenu } from "./new-session";
import {
  type AgentMap,
  type AgentSession,
  type PendingDetail,
  answerPending,
  type ChoiceQuestion,
  buildChoiceAnswer,
  describeToolInput,
  fetchPending,
  parseChoice,
  secondsLeft,
  applyEvent,
  fetchAgentSessions,
  forSession,
  replaceAll,
  stateText,
  waitingList,
} from "./agents";

const $ = (id: string) => document.getElementById(id)!;
const client = new OctoClient();
const TERM_CHANNEL = 1;

let sessions: any[] = [];
let attachedId: number | null = null;
let term: Terminal | null = null;
let fit: FitAddon | null = null;
let webgl: WebglAddon | null = null;
const previews = new Map<number, Terminal>();
/** key = `${agent_id} ${agent_session_id}`,见 agents.ts */
const agents: AgentMap = new Map();
/** 挂起请求的详情,key = pending id。广播里没有命令原文,详情单独拉(见 agents.ts)。 */
const pendingDetails = new Map<string, PendingDetail>();
/** 倒计时的刷新句柄。重绘时先清掉,免得叠出多个。 */
let countdownTimer: ReturnType<typeof setInterval> | null = null;
const BASE_TITLE = document.title;

let config: OctoConfig = loadConfig(resolveTheme);
// 语言先于任何一次渲染定下来:下面 mountSettings / mountNewSessionMenu 在模块
// 求值时就会铺一批静态文案,那时 t() 必须已经指向对的词条表。
applyLocale();

function token(): string {
  const fromHash = new URLSearchParams(location.hash.slice(1)).get("token");
  if (fromHash) localStorage.setItem("octoterm-token", fromHash);
  return localStorage.getItem("octoterm-token") ?? prompt(t("app.tokenPrompt")) ?? "";
}

function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${location.host}/ws`;
}

function fmtTime(unixSecs: number): string {
  // 跟界面语言走,而不是跟浏览器区域走:界面切成 English 之后还蹦出中文日期很怪
  return new Date(unixSecs * 1000).toLocaleString(localeTag());
}

/* ---------- 多语言 ---------- */

/**
 * 把配置里的语言偏好落到 i18n 上。"auto" 时按浏览器语言挑,所以每次配置变更
 * 都要重算——用户可能刚把「跟随浏览器」改成了固定语言。
 */
function applyLocale() {
  setLocale(resolveLocale(config.ui.locale, navigatorLanguages()));
}

/** index.html 里带 data-i18n / data-i18n-title 的静态文案。 */
function applyStaticText() {
  document.documentElement.lang = localeTag();
  for (const node of Array.from(document.querySelectorAll<HTMLElement>("[data-i18n]"))) {
    node.textContent = t(node.dataset.i18n as MsgKey);
  }
  for (const node of Array.from(document.querySelectorAll<HTMLElement>("[data-i18n-title]"))) {
    node.title = t(node.dataset.i18nTitle as MsgKey);
  }
}

/** 连接状态。存的是词条 key 而不是算好的字符串,这样切语言能原地重绘。 */
let connKey: MsgKey = "conn.connected";
function setConn(key: MsgKey) {
  connKey = key;
  $("conn-state").textContent = t(key);
}

/** 重连横幅。同上,存的是「怎么算出文案」——fatal 那条还带着服务端的原文参数。 */
let banner: (() => string) | null = null;
function setBanner(render: (() => string) | null) {
  banner = render;
  const el = $("reconnect-banner");
  el.hidden = render === null;
  if (render) el.textContent = render();
}

// 切语言:静态文案、侧边栏(会话时间也跟着换格式)、状态与横幅一起重绘。
// 设置面板和新建会话菜单各自订阅了自己的那部分。
subscribe(() => {
  applyStaticText();
  renderSidebar();
  setConn(connKey);
  setBanner(banner);
});

/* ---------- 外观配置 ---------- */

/** 主题/字体改动的唯一落点:界面变量、主终端、所有预览、渲染器,一次全刷。 */
function applyConfig(next: OctoConfig) {
  const previewToggled = next.ui.sidebarPreview !== config.ui.sidebarPreview;
  config = next;
  saveConfig(config);
  // 语言真变了才会通知订阅者(setLocale 对同值是空操作),所以不必自己比一遍
  applyLocale();
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
function applyRenderer(terminal: Terminal) {
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
    terminal.loadAddon(addon);
    webgl = addon;
  } catch (err) {
    console.warn("octoterm: WebGL 渲染器不可用,使用 DOM 渲染器", err);
    webgl = null;
  }
}

const settings = mountSettings({
  get: () => config,
  set: (next) => applyConfig(next),
  token,
});

/* ---------- sidebar:选择/预览/改名/结束都在这里 ---------- */
function renderSidebar() {
  const nav = $("session-nav");
  nav.innerHTML = "";
  previews.forEach((p) => p.dispose());
  previews.clear();
  if (sessions.length === 0) {
    nav.innerHTML = `<div class="empty">${t("session.empty")}</div>`;
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
        <button data-act="rename" title="${t("session.rename")}">✎</button>
        <button data-act="kill" title="${t("session.kill")}">✕</button>
      </div>`;
    row.querySelector(".sname")!.textContent = s.name;
    const agent = forSession(agents, s.id);
    if (agent) {
      // 一个字符的圆点太轻,扫一眼看不见 —— 这里要的是「哪台在等我」,
      // 所以做成有底色的胶囊,`waiting` 用最扎眼的一档(见 style.css)。
      row.querySelector(".sname")!.prepend(agentBadge(agent));
    }
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
        const name = prompt(t("session.renamePrompt"), s.name);
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
  $("session-list").hidden = true;
  $("back-to-list").hidden = false;
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
  $("session-list").hidden = false;
  $("back-to-list").hidden = true;
  renderSessionList();
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
// 回列表 = detach 并回到主视图。会话本身不受影响(协议 CH6:detach 从不结束会话),
// 再点进来是一次 resync —— 用一次重绘换「随时能纵览全局」,值。
//
// 顺手关掉抽屉:窄屏上侧边栏是盖在工作区上的浮层,不关掉的话点完这个按钮,
// 身后那张列表页恰好被挡住,看起来像什么都没发生。
$("back-to-list").addEventListener("click", () => {
  setDrawer(false);
  closeTerminal();
});
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
  // 断线期间漏掉的 agent-event 靠这次全量拉取补齐,不做增量对账
  void refreshAgents();
  setBanner(null);
  setConn("conn.connected");
  client.send({ type: "list-sessions" });
};
client.onReconnecting = () => {
  setBanner(() => t("conn.banner.reconnecting"));
  setConn("conn.reconnecting");
};
client.onFatal = (message) => {
  setBanner(() => t("conn.banner.fatal", { message }));
  setConn("conn.disconnected");
};
/** 状态胶囊。列表页和侧边栏共用同一个,保证两处看到的是一回事。 */
function agentBadge(a: AgentSession): HTMLElement {
  const pill = document.createElement("span");
  pill.className = `apill a-${a.state}`;
  pill.textContent = stateText(a);
  pill.title = stateText(a);
  return pill;
}

/**
 * 会话列表页。
 *
 * 侧边栏那份是「随时能切」的窄条,这一份是「站在这里挑」的主视图 —— 卡片更大、
 * 状态更显眼、还带预览。没有会话时它就是空状态提示,不再单独留一个 empty 元素。
 */
function renderSessionList() {
  const box = $("session-list");
  box.innerHTML = "";
  if (attachedId !== null) return; // 正在看终端时不渲染,省得白算
  if (sessions.length === 0) {
    box.innerHTML = `<div class="empty">${t("app.empty")}</div>`;
    return;
  }
  const title = document.createElement("h2");
  title.className = "slist-title";
  title.textContent = t("session.listTitle");
  box.appendChild(title);

  const grid = document.createElement("div");
  grid.className = "slist-grid";
  for (const s of sessions) {
    const card = document.createElement("div");
    card.className = "scard";
    const head = document.createElement("div");
    head.className = "scard-head";
    const name = document.createElement("span");
    name.className = "scard-name";
    name.textContent = s.name;
    head.appendChild(name);
    const agent = forSession(agents, s.id);
    if (agent) head.appendChild(agentBadge(agent));
    const meta = document.createElement("div");
    meta.className = "scard-meta";
    meta.textContent = `${s.cols}×${s.rows} · ${fmtTime(s.created_at)}`;
    const open = document.createElement("button");
    open.className = "scard-open";
    open.textContent = t("session.open");
    open.addEventListener("click", () => openTerminal(s.id));
    card.append(head, meta, open);
    // 整张卡都可点,按钮只是给「这里可以点」一个明确的落点
    card.addEventListener("click", (ev) => {
      if ((ev.target as HTMLElement).tagName !== "BUTTON") openTerminal(s.id);
    });
    grid.appendChild(card);
  }
  box.appendChild(grid);
}

/**
 * 标签页标题上的待办数。
 *
 * 这是最便宜的跨设备注意力机制:这功能的卖点就是「在手机上接管」,而手机浏览器
 * 十有八九把它压在后台 —— 页面里做得再显眼也看不见,标题栏能。
 */
function updateTitleBadge() {
  const n = waitingList(agents).length;
  document.title = n > 0 ? `(${n}) ${BASE_TITLE}` : BASE_TITLE;
}

/**
 * 「有 AI 在等你」横幅。
 *
 * 设计上只有一条铁律:**不让人盲签**。第一版只显示「会话名 · 等你回答」,等于让
 * 用户一键批准一条看不见的命令 —— 那比没有这个功能更危险,它会训练人闭眼点允许。
 * 所以命令原文必须在按钮**上方**、完整、不截断。
 *
 * 这里只放结构化的允许/拒绝。自由文本回答不在这儿 —— octoterm 托管着那个 pty,
 * 「去这个会话」一键过去在终端里打字就是了,再造一个输入框既多余又处理不了
 * agent 自己画的 TUI 交互。
 */
function renderAgentBanner() {
  const box = $("agent-banner");
  if (countdownTimer) {
    clearInterval(countdownTimer);
    countdownTimer = null;
  }
  updateTitleBadge();

  const waiting = waitingList(agents);
  if (waiting.length === 0) {
    box.hidden = true;
    box.innerHTML = "";
    return;
  }
  box.hidden = false;
  box.innerHTML = "";
  const title = document.createElement("div");
  title.className = "abanner-title";
  title.textContent = t("agent.waitingTitle");
  box.appendChild(title);

  const ticks: { el: HTMLElement; expiresAt: number }[] = [];

  for (const a of waiting) {
    const detail = pendingDetails.get(a.pending!);
    const row = document.createElement("div");
    row.className = "abanner-row";

    // 第一行:哪个会话、什么工具、还剩多久
    const head = document.createElement("div");
    head.className = "abanner-head";
    const host = sessions.find((x) => x.id === a.session);
    const who = document.createElement("span");
    who.className = "abanner-who";
    who.textContent = `${host ? host.name : `#${a.session}`} · ${
      detail?.tool_name ?? a.detail ?? t("agent.unknownTool")
    }`;
    head.appendChild(who);
    if (detail) {
      const left = document.createElement("span");
      left.className = "abanner-left";
      ticks.push({ el: left, expiresAt: detail.expires_at });
      head.appendChild(left);
    }
    row.appendChild(head);

    // 选择题走另一套:它要的不是「准不准」,而是「选哪个」
    const choice = detail ? parseChoice(detail.tool_name, detail.tool_input) : null;
    if (choice) {
      row.appendChild(renderChoice(choice, a, detail!));
      box.appendChild(row);
      continue;
    }

    // 第二行:提醒 + 命令原文。**这两行是这个横幅存在的理由**
    if (detail) {
      const hint = document.createElement("div");
      hint.className = "abanner-hint";
      hint.textContent = t("agent.reviewHint");
      const cmd = document.createElement("pre");
      cmd.className = "abanner-cmd";
      cmd.textContent = describeToolInput(detail.tool_input);
      row.append(hint, cmd);
    }

    // 第三行:拒绝理由 + 按钮
    const acts = document.createElement("div");
    acts.className = "abanner-acts";
    const reason = document.createElement("input");
    reason.className = "abanner-reason";
    reason.type = "text";
    reason.placeholder = t("agent.denyReason");
    const status = document.createElement("span");
    status.className = "abanner-status";

    const submit = async (decision: "allow" | "deny", btns: HTMLButtonElement[]) => {
      btns.forEach((b) => (b.disabled = true));
      const msg = reason.value.trim() || undefined;
      const outcome = await answerPending(token(), a.pending!, decision, msg);
      if (outcome !== "ok") {
        status.textContent =
          outcome === "gone" ? t("agent.gone")
          : outcome === "already" ? t("agent.already")
          : t("agent.failed");
      }
      pendingDetails.delete(a.pending!);
      // 不管结果如何都先摘掉本地这条;服务端会用 agent-event 把真相推回来
      a.pending = null;
      renderAgentBanner();
      renderSidebar();
    };

    // 拒绝排在前面、样式朴素;允许在后面、带强调色。
    // 一个可能执行 rm -rf 的动作不该和「算了」视觉等重,更不该是顺手就点到的那个。
    const deny = document.createElement("button");
    deny.textContent = t("agent.deny");
    const allow = document.createElement("button");
    allow.className = "abanner-allow";
    allow.textContent = t("agent.allow");
    deny.addEventListener("click", () => submit("deny", [deny, allow]));
    allow.addEventListener("click", () => submit("allow", [deny, allow]));

    const go = document.createElement("button");
    go.textContent = t("agent.openSession");
    go.addEventListener("click", () => {
      if (a.session != null) openTerminal(a.session);
    });

    acts.append(reason, deny, allow, go, status);
    row.appendChild(acts);
    box.appendChild(row);
  }

  // 倒计时只改那一个 span,不整体重绘 —— 重绘会把用户正在写的拒绝理由清掉
  if (ticks.length > 0) {
    const paint = () => {
      const now = Math.floor(Date.now() / 1000);
      for (const { el, expiresAt } of ticks) {
        const left = secondsLeft(expiresAt, now);
        el.textContent = left > 0 ? t("agent.expiresIn", { n: left }) : t("agent.expired");
        el.classList.toggle("is-expired", left === 0);
      }
    };
    paint();
    countdownTimer = setInterval(paint, 1000);
  }
}

/**
 * 选择题的界面。
 *
 * 和授权那套的区别是它没有「准/不准」—— 只有「选哪个」。所以按钮是选项本身,
 * 全部问题都选完才允许提交(半份答案回传过去,agent 拿到的是一份残缺的入参)。
 *
 * 问题原文**不做 JS 截断**:回传的 `answers` 以问题原文为键,一旦拿截断后的文本
 * 当键,答案会对不上任何一个问题而被静默丢掉。要省略号让 CSS 去画。
 */
function renderChoice(
  questions: ChoiceQuestion[],
  a: AgentSession,
  detail: PendingDetail,
): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "achoice";
  // 横幅的大标题是通用的「有 AI 在等你回答」;这里要说清楚这一条**不是授权**,
  // 是在问你 —— 两者的按钮语义完全不同,混起来会让人以为点了就等于放行。
  const kind = document.createElement("div");
  kind.className = "achoice-kind";
  kind.textContent = t("agent.choiceTitle");
  wrap.appendChild(kind);
  const picked: Record<string, string> = {};

  const submit = document.createElement("button");
  submit.className = "abanner-allow";
  submit.textContent = t("agent.choiceSubmit");
  submit.disabled = true;
  const status = document.createElement("span");
  status.className = "abanner-status";
  status.textContent = t("agent.choicePending");

  const syncSubmit = () => {
    const done = questions.every((q) => picked[q.question]);
    submit.disabled = !done;
    status.textContent = done ? "" : t("agent.choicePending");
  };

  for (const q of questions) {
    const block = document.createElement("div");
    block.className = "achoice-q";
    const label = document.createElement("div");
    label.className = "achoice-question";
    label.textContent = q.header ? `${q.header} — ${q.question}` : q.question;
    label.title = q.question;
    block.appendChild(label);

    const opts = document.createElement("div");
    opts.className = "achoice-opts";
    for (const o of q.options) {
      const b = document.createElement("button");
      b.className = "achoice-opt";
      b.textContent = o.label;
      if (o.description) b.title = o.description;
      b.addEventListener("click", () => {
        picked[q.question] = o.label;
        opts.querySelectorAll(".achoice-opt").forEach((x) => x.classList.remove("is-picked"));
        b.classList.add("is-picked");
        syncSubmit();
      });
      opts.appendChild(b);
    }
    block.appendChild(opts);
    wrap.appendChild(block);
  }

  submit.addEventListener("click", async () => {
    submit.disabled = true;
    const outcome = await answerPending(
      token(),
      a.pending!,
      "allow",
      undefined,
      buildChoiceAnswer(detail.tool_input, picked),
    );
    if (outcome !== "ok") {
      status.textContent =
        outcome === "gone" ? t("agent.gone")
        : outcome === "already" ? t("agent.already")
        : t("agent.failed");
    }
    pendingDetails.delete(a.pending!);
    a.pending = null;
    renderAgentBanner();
    renderSidebar();
  });

  const acts = document.createElement("div");
  acts.className = "abanner-acts";
  const go = document.createElement("button");
  go.textContent = t("agent.openSession");
  go.addEventListener("click", () => {
    if (a.session != null) openTerminal(a.session);
  });
  acts.append(submit, go, status);
  wrap.appendChild(acts);
  return wrap;
}

/**
 * 补齐挂起详情。广播只说「有事了」,命令原文得单独拉一次(协议 R4:控制通道不走
 * 大块数据)。拉完重绘一次。
 */
async function refreshPendingDetails() {
  const want = waitingList(agents).map((a) => a.pending!);
  if (want.length === 0) {
    pendingDetails.clear();
    return;
  }
  if (want.every((id) => pendingDetails.has(id))) return; // 已经齐了,别白跑
  for (const p of await fetchPending(token())) pendingDetails.set(p.id, p);
  renderAgentBanner();
}

/** 全量拉取。页面打开和每次重连后都要做一次(协议 A5)。 */
async function refreshAgents() {
  replaceAll(agents, (await fetchAgentSessions(token())) as AgentSession[]);
  renderSidebar();
  renderSessionList();
  renderAgentBanner();
  void refreshPendingDetails();
}

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
      renderSessionList();
      break;
    case "session-event":
      if (msg.event === "closed" && attachedId === msg.session?.id) {
        closeTerminal();
      }
      client.send({ type: "list-sessions" });
      break;
    case "agent-event":
      applyEvent(agents, msg as AgentSession);
      renderSidebar();
      renderSessionList();
      renderAgentBanner();
      void refreshPendingDetails();
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

// 界面配色和文案先于连接落地:否则首帧会闪一下 style.css 里那套写死的默认色,
// 以及 index.html 里那些还空着的 data-i18n 节点
applyUiColors(config);
applyStaticText();
setConn(connKey);
// 只取一次:token() 在 localStorage 里没有时会 prompt,取两次就要问两遍
const authToken = token();
client.connect(wsUrl(), authToken);
