import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { execSync } from "node:child_process";

const here = dirname(fileURLToPath(import.meta.url));
execSync("npx esbuild src/chat.ts --bundle --format=esm --outfile=test/.chat.build.mjs", {
  cwd: join(here, ".."),
});
const { mergeWindow, fetchMessages, fallbackText, preview } = await import("./.chat.build.mjs");

const msg = (id, text = "hi") => ({ id, role: "assistant", ts: null, blocks: [{ kind: "text", text }] });
const win = (messages, over = {}) => ({ source: "transcript", messages, cursor: "c", reset: false, more: false, ...over });

test("追加时按 id 去重 —— 窗口边界上那条可能被重复送来", () => {
  const prev = [msg("a"), msg("b")];
  const out = mergeWindow(prev, win([msg("b"), msg("c")]));
  assert.deepEqual(out.map((m) => m.id), ["a", "b", "c"]);
});

/// reset 意味着服务端认定旧游标失效(compact 了、或换了会话)。那时旧内容和新内容
/// 不是同一条时间线,接在一起会得到一段前后不搭的对话。
test("reset 是整段替换,不是追加", () => {
  const prev = [msg("a"), msg("b")];
  const out = mergeWindow(prev, win([msg("z")], { reset: true }));
  assert.deepEqual(out.map((m) => m.id), ["z"]);
});

test("回落时不动已有消息", () => {
  const prev = [msg("a")];
  assert.deepEqual(mergeWindow(prev, { source: "terminal", reason: "disabled" }), prev);
});

test("路由缺失/报错降级为 terminal 而不是抛(协议 T12)", async () => {
  globalThis.fetch = async () => ({ ok: false, status: 404 });
  assert.equal((await fetchMessages("t", "claude-code", "s1")).source, "terminal");
  globalThis.fetch = async () => { throw new Error("offline"); };
  assert.equal((await fetchMessages("t", "claude-code", "s1")).source, "terminal");
});

test("游标作为 after 参数带上", async () => {
  let seen;
  globalThis.fetch = async (url) => { seen = url; return { ok: true, json: async () => win([]) }; };
  await fetchMessages("t", "claude-code", "s 1", "12.34");
  assert.match(seen, /agent_id=claude-code/);
  assert.match(seen, /agent_session_id=s\+1/);
  assert.match(seen, /after=12\.34/);
});

test("认不出的回落原因也要说人话,不能把机器键怼给用户", () => {
  const known = fallbackText("disabled");
  const unknown = fallbackText("something-new-from-the-future");
  assert.ok(!known.includes("disabled"));
  assert.ok(!unknown.includes("something-new"));
  assert.ok(unknown.length > 0);
});

test("preview 优先取正文,没有正文就取工具调用", () => {
  assert.equal(preview(msg("a", "正文")), "正文");
  const toolOnly = { id: "t", role: "assistant", ts: null, blocks: [{ kind: "tool-use", name: "Bash", input: "ls" }] };
  assert.equal(preview(toolOnly), "Bash ls");
});
