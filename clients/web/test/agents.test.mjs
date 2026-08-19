import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { execSync } from "node:child_process";

// 同 launchers.test.mjs:node 跑不了 .ts,用 esbuild 即时转译。
const here = dirname(fileURLToPath(import.meta.url));
execSync("npx esbuild src/agents.ts --bundle --format=esm --outfile=test/.agents.build.mjs", {
  cwd: join(here, ".."),
});
const {
  applyEvent, replaceAll, forSession, waitingList, keyOf, answerPending, fetchAgentSessions,
  summarizePlan, fetchAgentPlan,
} = await import("./.agents.build.mjs");

const ev = (over = {}) => ({
  agent_id: "claude-code",
  agent_session_id: "s1",
  session: 1,
  state: "working",
  detail: null,
  pending: null,
  ...over,
});

test("同一个 agent 会话的后续事件是覆盖而不是追加", () => {
  const m = new Map();
  applyEvent(m, ev({ state: "thinking" }));
  applyEvent(m, ev({ state: "working" }));
  assert.equal(m.size, 1);
  assert.equal(m.get(keyOf(ev())).state, "working");
});

test("done 的会话从表里移除,不留在列表上误导用户", () => {
  const m = new Map();
  applyEvent(m, ev());
  applyEvent(m, ev({ state: "done" }));
  assert.equal(m.size, 0);
});

test("全量替换时把 done 的滤掉", () => {
  const m = new Map();
  replaceAll(m, [ev(), ev({ agent_session_id: "s2", state: "done" })]);
  assert.equal(m.size, 1);
});

test("一个托管会话里有多个 agent 时,waiting 优先被显示", () => {
  const m = new Map();
  applyEvent(m, ev({ agent_session_id: "a", state: "working" }));
  applyEvent(m, ev({ agent_session_id: "b", state: "waiting", pending: "p1" }));
  assert.equal(forSession(m, 1).state, "waiting");
});

test("forSession 只看自己那个托管会话", () => {
  const m = new Map();
  applyEvent(m, ev({ session: 2, state: "waiting", pending: "p1" }));
  assert.equal(forSession(m, 1), null);
  assert.equal(forSession(m, 2).state, "waiting");
});

test("waitingList 只收有 pending 的,并按托管会话 id 排序", () => {
  const m = new Map();
  applyEvent(m, ev({ agent_session_id: "a", session: 3, state: "waiting", pending: "p3" }));
  applyEvent(m, ev({ agent_session_id: "b", session: 1, state: "waiting", pending: "p1" }));
  applyEvent(m, ev({ agent_session_id: "c", session: 2, state: "working" }));
  assert.deepEqual(waitingList(m).map((s) => s.pending), ["p1", "p3"]);
});

test("路由不存在时降级为空数组而不是抛(协议 T12)", async () => {
  globalThis.fetch = async () => ({ ok: false, status: 404 });
  assert.deepEqual(await fetchAgentSessions("tok"), []);
  globalThis.fetch = async () => { throw new Error("offline"); };
  assert.deepEqual(await fetchAgentSessions("tok"), []);
});

test("回答的三种失败要能分开:过期 / 别人先答了 / 送不到", async () => {
  const codes = { 404: "gone", 409: "already", 500: "failed" };
  for (const [status, expected] of Object.entries(codes)) {
    globalThis.fetch = async () => ({ ok: false, status: Number(status) });
    assert.equal(await answerPending("tok", "p1", "allow"), expected);
  }
  globalThis.fetch = async () => ({ ok: true, status: 200 });
  assert.equal(await answerPending("tok", "p1", "allow"), "ok");
});

test("回答请求带上 bearer 与自然键", async () => {
  let seen;
  globalThis.fetch = async (url, init) => { seen = { url, init }; return { ok: true, status: 200 }; };
  await answerPending("tok", "p9", "deny", "算了");
  assert.equal(seen.url, "/api/agents/answer");
  assert.equal(seen.init.method, "POST");
  assert.equal(seen.init.headers.Authorization, "Bearer tok");
  assert.deepEqual(JSON.parse(seen.init.body), { pending_id: "p9", decision: "deny", message: "算了" });
});

test("预演按文件归并,一个文件一行,事件列在后面", () => {
  const edits = [
    { path: "/h/.claude/settings.json", action: "ensure", event: "Stop", spec: {} },
    { path: "/h/.claude/settings.json", action: "ensure", event: "PreToolUse", spec: {} },
  ];
  assert.deepEqual(summarizePlan(edits), [
    { path: "/h/.claude/settings.json", events: ["Stop", "PreToolUse"] },
  ]);
});

test("预演拉不到时返回 null,让 UI 显示读不到而不是空白", async () => {
  globalThis.fetch = async () => ({ ok: false, status: 500 });
  assert.equal(await fetchAgentPlan("tok", "claude-code"), null);
  globalThis.fetch = async () => { throw new Error("offline"); };
  assert.equal(await fetchAgentPlan("tok", "claude-code"), null);
});

test("agent id 进 URL 前要转义", async () => {
  let seen;
  globalThis.fetch = async (url) => { seen = url; return { ok: true, json: async () => ({}) }; };
  await fetchAgentPlan("tok", "a/b c");
  assert.equal(seen, "/api/agents/a%2Fb%20c/plan");
});
