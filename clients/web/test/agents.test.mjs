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
  summarizePlan, fetchAgentPlan, describeToolInput, secondsLeft, stateText, fetchPending,
  parseChoice, buildChoiceAnswer,
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

test("命令原文要原样拿出来 —— 看不清就是盲签", () => {
  assert.equal(describeToolInput({ command: "rm -rf build/" }), "rm -rf build/");
  assert.equal(describeToolInput({ file_path: "/etc/hosts" }), "/etc/hosts");
  assert.equal(describeToolInput("plain string"), "plain string");
});

test("认不出的入参也要显示,而不是显示空白", () => {
  const out = describeToolInput({ weird: 1, nested: { a: 2 } });
  assert.match(out, /weird/);
  assert.match(out, /nested/);
  assert.equal(describeToolInput(null), "");
});

test("倒计时不会变成负数", () => {
  assert.equal(secondsLeft(100, 40), 60);
  assert.equal(secondsLeft(100, 100), 0);
  assert.equal(secondsLeft(100, 999), 0);
});

/// waiting 的两种来源必须说成不同的话:有 pending 的「这里能回答」,
/// 没有的是 Notification 报来的「它在终端那边等你」。
test("两种 waiting 的文案不同", () => {
  const withPending = ev({ state: "waiting", pending: "p1" });
  const noPending = ev({ state: "waiting", pending: null });
  assert.notEqual(stateText(withPending), stateText(noPending));
});

test("notification_type 这种机器串要翻译,工具名原样显示", () => {
  const notice = stateText(ev({ state: "waiting", pending: null, detail: "permission_prompt" }));
  assert.ok(!notice.includes("permission_prompt"), `机器串漏到界面上了: ${notice}`);
  const tool = stateText(ev({ state: "working", detail: "Bash" }));
  assert.ok(tool.includes("Bash"), "工具名应当原样显示");
});

test("挂起详情拉不到时降级为空数组(协议 T12)", async () => {
  globalThis.fetch = async () => ({ ok: false, status: 404 });
  assert.deepEqual(await fetchPending("tok"), []);
});

const ask = (questions) => ({ questions });

test("只有 AskUserQuestion 才当选择题", () => {
  const input = ask([{ question: "选哪个?", options: [{ label: "A" }] }]);
  assert.equal(parseChoice("Bash", input), null);
  assert.equal(parseChoice("AskUserQuestion", input).length, 1);
});

test("形状不对或超限一律不渲染 —— 绝不半渲染", () => {
  const bad = [
    ask([]),
    ask([{ question: "q", options: [] }]),
    ask([{ question: "q" }]),
    ask([{ options: [{ label: "A" }] }]),
    ask([{ question: "q", options: [{ nolabel: 1 }] }]),
    ask(Array.from({ length: 6 }, (_, i) => ({ question: `q${i}`, options: [{ label: "A" }] }))),
    ask([{ question: "q", options: Array.from({ length: 9 }, (_, i) => ({ label: `o${i}` })) }]),
  ];
  for (const input of bad) {
    assert.equal(parseChoice("AskUserQuestion", input), null, `不该渲染: ${JSON.stringify(input)}`);
  }
});

/// 两道问题文本相同的话,以问题原文为键的 answers 会互相覆盖。
test("重复的问题文本不渲染", () => {
  const dup = ask([
    { question: "同一句", options: [{ label: "A" }] },
    { question: "同一句", options: [{ label: "B" }] },
  ]);
  assert.equal(parseChoice("AskUserQuestion", dup), null);
});

/// 键必须是问题原文。界面上截断显示是很自然的念头,但拿截断文本当键,
/// 答案会对不上任何一个问题、被 agent 静默丢掉。
test("答案以问题原文为键,并保留原入参的其它字段", () => {
  const input = { questions: [{ question: "很长很长的一句问题原文", options: [] }], extra: 1 };
  const out = buildChoiceAnswer(input, { "很长很长的一句问题原文": "SQLite" });
  assert.equal(out.answers["很长很长的一句问题原文"], "SQLite");
  assert.equal(out.extra, 1, "原入参的其它字段要原样带回去");
  assert.ok(Array.isArray(out.questions));
});
