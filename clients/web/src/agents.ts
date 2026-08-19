/**
 * 托管会话里跑着的 coding agent。
 *
 * 两件事:在会话列表上标出它在干什么,以及在它等人拍板时让用户就地回答。
 *
 * 状态的**增量**来自 `agent-event` 控制消息广播,**全量**来自
 * `GET /api/agents/sessions` —— 页面加载时和每次重连后都要拉一次全量(协议 A5:
 * 不做增量对账,断线期间漏掉的事件靠这次拉取补齐)。
 *
 * 回答走 `POST /api/agents/answer` 而不是控制消息:新增 client→server 消息类型
 * 按协议 X3 是破坏性变更,要 bump proto 并让所有已打开的页面全断。
 *
 * 纯逻辑(状态归并、取哪个 agent 会话代表这个终端、状态图标)与 DOM 分开,
 * 这样它们能被 node:test 直接跑。
 */
import { type MsgKey, t } from "./i18n";

export type AgentState = "idle" | "thinking" | "working" | "waiting" | "done" | "error";

export interface AgentSession {
  agent_id: string;
  agent_session_id: string;
  /** 关联到的 octoterm 托管会话 id */
  session: number | null;
  state: AgentState;
  detail: string | null;
  /** 有值 = 正在等人回答,值是回答时要带的自然键 */
  pending: string | null;
}

export type AgentMap = Map<string, AgentSession>;

export function keyOf(s: { agent_id: string; agent_session_id: string }): string {
  return `${s.agent_id} ${s.agent_session_id}`;
}

/**
 * 把一条 `agent-event` 并进表里。
 *
 * `done` 的会话直接移除:它已经结束了,留在列表上只会让用户以为还有东西在跑。
 * 服务端的清理也会把它扫掉,这里先一步只是为了界面即时。
 */
export function applyEvent(map: AgentMap, ev: AgentSession): AgentMap {
  if (ev.state === "done") map.delete(keyOf(ev));
  else map.set(keyOf(ev), ev);
  return map;
}

export function replaceAll(map: AgentMap, list: AgentSession[]): AgentMap {
  map.clear();
  for (const s of list) if (s.state !== "done") map.set(keyOf(s), s);
  return map;
}

/**
 * 状态优先级。一个托管会话里可能同时有多个 agent 会话(开了 subagent,或者用户
 * 在同一个终端里换了个 agent),列表上只有一个位置,得挑一个最该被看见的。
 *
 * `waiting` 永远排第一 —— 它是唯一需要用户**动手**的状态。
 */
const PRIORITY: Record<AgentState, number> = {
  waiting: 5,
  error: 4,
  working: 3,
  thinking: 2,
  idle: 1,
  done: 0,
};

/** 这个托管会话该显示哪个 agent 的状态。没有 agent 就返回 null。 */
export function forSession(map: AgentMap, sessionId: number): AgentSession | null {
  let best: AgentSession | null = null;
  for (const s of map.values()) {
    if (s.session !== sessionId) continue;
    if (!best || PRIORITY[s.state] > PRIORITY[best.state]) best = s;
  }
  return best;
}

/** 所有正在等人回答的。按托管会话 id 排序,让列表稳定不跳。 */
export function waitingList(map: AgentMap): AgentSession[] {
  return [...map.values()]
    .filter((s) => s.pending)
    .sort((a, b) => (a.session ?? 0) - (b.session ?? 0));
}


/**
 * 状态 → 词条键。**显式写全**,不用模板字面量拼 —— 拼出来的键既躲开了 `MsgKey`
 * 的类型检查(只能 `as never`),也躲开了「没人引用的死词条」那条测试,等于把两道
 * 防线一起绕过去了。
 */
const STATE_KEY: Record<AgentState, MsgKey> = {
  idle: "agent.state.idle",
  thinking: "agent.state.thinking",
  working: "agent.state.working",
  waiting: "agent.state.waiting",
  done: "agent.state.done",
  error: "agent.state.error",
};

/** 给人看的一行:状态名 +(如果有)细节。细节原样显示,不解析。 */
export function stateText(s: AgentSession): string {
  const name = t(STATE_KEY[s.state]);
  return s.detail ? `${name} · ${s.detail}` : name;
}

/* ---------- HTTP ---------- */

function authHeaders(token: string): HeadersInit {
  return { Authorization: `Bearer ${token}` };
}

/**
 * 全量快照。任何一步失败都返回空数组而不是抛 —— 服务端可能是没有这个路由的
 * 旧版本(协议 T12:客户端必须容忍 `/api/` 路由缺失,降级而不是卡住)。
 */
export async function fetchAgentSessions(token: string): Promise<AgentSession[]> {
  try {
    const r = await fetch("/api/agents/sessions", { headers: authHeaders(token) });
    if (!r.ok) return [];
    const body = await r.json();
    return Array.isArray(body?.sessions) ? body.sessions : [];
  } catch {
    return [];
  }
}

export type AnswerOutcome = "ok" | "gone" | "already" | "failed";

/**
 * 替 agent 拍板。
 *
 * 三种失败要分开:`gone` = 这个请求已经不在了(agent 自己超时了或断了),
 * `already` = 别的客户端抢先答过了(多设备同时看着同一台机器,这是常态,
 * 不是错误)。
 */
export async function answerPending(
  token: string,
  pendingId: string,
  decision: "allow" | "deny",
  message?: string,
): Promise<AnswerOutcome> {
  try {
    const r = await fetch("/api/agents/answer", {
      method: "POST",
      headers: { ...authHeaders(token), "Content-Type": "application/json" },
      body: JSON.stringify({ pending_id: pendingId, decision, message }),
    });
    if (r.ok) return "ok";
    if (r.status === 404) return "gone";
    if (r.status === 409) return "already";
    return "failed";
  } catch {
    return "failed";
  }
}

/* ---------- 安装(设置页用) ---------- */

export interface AgentStatus {
  id: string;
  name: string;
  detected: { installed: boolean; confidence: string; reason: string; detail: string };
  /** "not-installed" | "installed" | "stale-port" */
  integration: string;
  /**
   * 装完还需要用户做什么才生效。机器可读的键,文案在客户端。
   * `null` = 装完即生效。
   */
  activation: string | null;
  /** 同一事件上别家的阻塞式 hook。不是错误,但要让用户看见 */
  conflicts: string[];
}

/** 路由缺失/报错一律返回空清单(协议 T12),设置页显示「没找到」而不是崩掉。 */
export async function fetchAgents(token: string): Promise<AgentStatus[]> {
  try {
    const r = await fetch("/api/agents", { headers: authHeaders(token) });
    if (!r.ok) return [];
    const body = await r.json();
    return Array.isArray(body?.agents) ? body.agents : [];
  } catch {
    return [];
  }
}

export type InstallOutcome = "ok" | "disabled" | "conflict" | "failed";

/**
 * 装 / 卸 hook。
 *
 * `disabled`(403)不是错误,是「服务端把这个能力关着」—— 它默认就是关的,因为
 * 这个动作会去改用户的 `~/.claude/settings.json`。UI 要把它当成一句说明,
 * 而不是一条失败。
 */
export async function setAgentIntegration(
  token: string,
  id: string,
  install: boolean,
): Promise<InstallOutcome> {
  try {
    const r = await fetch(`/api/agents/${encodeURIComponent(id)}/${install ? "install" : "uninstall"}`, {
      method: "POST",
      headers: authHeaders(token),
    });
    if (r.ok) return "ok";
    if (r.status === 403) return "disabled";
    if (r.status === 409) return "conflict";
    return "failed";
  } catch {
    return "failed";
  }
}

/** 一条将要发生的改动。`spec` 是要写进去的 hook 原文,预演时原样展示。 */
export interface PlanEdit {
  path: string;
  action: "ensure" | "remove";
  event: string;
  spec: unknown;
}

export interface AgentPlan {
  install: PlanEdit[];
  uninstall: PlanEdit[];
  /** 是否包含决策类 hook。检测到别家阻塞式 hook 时服务端会自动置 false */
  include_blocking: boolean;
  install_enabled: boolean;
}

/**
 * 预演:装这一下到底会改什么。
 *
 * 这是只读的,和 install 走同一份计划 —— 「先看后装」不是另一条代码路径,而是
 * 同一条路径的干跑。改的是**用户的**配置文件,不给看一眼就动手是不合适的。
 */
export async function fetchAgentPlan(token: string, id: string): Promise<AgentPlan | null> {
  try {
    const r = await fetch(`/api/agents/${encodeURIComponent(id)}/plan`, {
      headers: authHeaders(token),
    });
    if (!r.ok) return null;
    return (await r.json()) as AgentPlan;
  } catch {
    return null;
  }
}

/**
 * 把计划压成给人看的几行:哪个文件、涉及哪些事件。
 *
 * 不展开 hook 的完整 JSON —— 那是给机器看的,一屏塞不下,而用户真正要判断的是
 * 「你要动我哪个文件、动几处」。
 */
export function summarizePlan(edits: PlanEdit[]): { path: string; events: string[] }[] {
  const byPath = new Map<string, string[]>();
  for (const e of edits) {
    const list = byPath.get(e.path) ?? [];
    list.push(e.event);
    byPath.set(e.path, list);
  }
  return [...byPath.entries()].map(([path, events]) => ({ path, events }));
}
