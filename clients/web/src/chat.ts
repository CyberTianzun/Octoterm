/**
 * 聊天视图的数据层。
 *
 * 消息来自 agent 自己写的对话记录(服务端读、归一化,见 `agent/transcript`),走 HTTP
 * 拉取 —— 一段对话可以是几 MB,协议 R4 不许在控制通道走大块数据。**增量由
 * `agent-event` 触发**:有事件才可能有新消息,所以不做轮询。
 *
 * 纯逻辑与 DOM 分开,和 `agents.ts` 同规矩:合并、去重能被 node:test 直接跑。
 */
import { type MsgKey, t } from "./i18n";

export type Role = "user" | "assistant" | "system";

export type Block =
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string }
  | { kind: "tool-use"; name: string; input: string }
  | { kind: "tool-result"; ok: boolean; text: string };

export interface Message {
  id: string;
  role: Role;
  ts: number | null;
  blocks: Block[];
}

/** 读不到对话记录时的原因。服务端给的是机器可读的键,文案在这里。 */
export type FallbackReason =
  | "disabled"
  | "no-transcript-path"
  | "unsupported-agent"
  | "unreadable"
  | "parse-failed";

export type ChatWindow =
  | { source: "transcript"; messages: Message[]; cursor: string; reset: boolean; more: boolean }
  | { source: "terminal"; reason: FallbackReason };

const FALLBACK_KEY: Record<FallbackReason, MsgKey> = {
  disabled: "chat.fallback.disabled",
  "no-transcript-path": "chat.fallback.noPath",
  "unsupported-agent": "chat.fallback.unsupported",
  unreadable: "chat.fallback.unreadable",
  "parse-failed": "chat.fallback.parseFailed",
};

/** 认不出的原因也要说人话,而不是把机器键怼给用户。 */
export function fallbackText(reason: string): string {
  const key = FALLBACK_KEY[reason as FallbackReason];
  return key ? t(key) : t("chat.fallback.unreadable");
}

export async function fetchMessages(
  token: string,
  agentId: string,
  agentSessionId: string,
  after?: string,
): Promise<ChatWindow> {
  const q = new URLSearchParams({ agent_id: agentId, agent_session_id: agentSessionId });
  if (after) q.set("after", after);
  try {
    const r = await fetch(`/api/agents/messages?${q}`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    // 老版本服务端没有这个路由。降级,不是报错(协议 T12)。
    if (!r.ok) return { source: "terminal", reason: "unreadable" };
    return (await r.json()) as ChatWindow;
  } catch {
    return { source: "terminal", reason: "unreadable" };
  }
}

/**
 * 把一次拉取合并进已有的消息。
 *
 * `reset` 是**整段替换**而不是追加:它意味着服务端认定旧游标失效(记录被 compact 了、
 * 或者换了个会话),那时旧内容和新内容不是同一条时间线,接在一起会得到一段前后不搭的
 * 对话。
 *
 * 追加时按 id 去重:窗口边界上的那条可能被重复送来。
 */
export function mergeWindow(prev: Message[], win: ChatWindow): Message[] {
  if (win.source !== "transcript") return prev;
  if (win.reset) return win.messages;
  const seen = new Set(prev.map((m) => m.id));
  return prev.concat(win.messages.filter((m) => !seen.has(m.id)));
}

/** 一条消息的纯文本预览,给折叠态和无障碍用。 */
export function preview(m: Message): string {
  for (const b of m.blocks) {
    if (b.kind === "text") return b.text;
    if (b.kind === "tool-use") return `${b.name} ${b.input}`;
  }
  return "";
}
