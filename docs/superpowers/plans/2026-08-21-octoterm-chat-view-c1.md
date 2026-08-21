# octoterm 聊天视图实施计划(C1:只读)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 托管会话里跑着 Claude Code 时,客户端可以用**聊天视图**打开它 —— 结构化的
消息流(说了什么、想了什么、调了哪个工具、结果如何),而不是一块 80×24 的 VT 画面。
C1 **只读**:发消息是 C2。

**Architecture:** `transcript_path` 我们每次 hook 都收到、一直扔掉,现在存下来。服务端
按窗口 + 游标读那个 JSONL,由 adapter 归一化成**客户端中立**的消息模型,走 HTTP 出去
(不进控制通道 —— 一段对话可以几 MB,协议 R4 明令不许)。增量靠已有的 `agent-event`
触发拉取,不新增推送机制,不动 proto。读不到就带**类型化的原因**回落到终端视图。

**Tech Stack:** Rust 2024 / axum 0.8 / serde_json —— **不新增依赖**。

设计依据:`docs/superpowers/specs/2026-08-21-octoterm-chat-view-design.md`

## Global Constraints

- **C1 只做 Claude Code,只做只读**。Codex / Grok 是 C3,发消息是 C2。
- **不 bump `PROTO_VERSION`**。只新增 `/api/` 只读路由(T12 允许),不加控制消息。
- **默认关**。`agents.transcript_enabled` 默认 `false` —— 装 hook 是一个决定,把整段
  对话送上网是另一个决定,后者不能靠前者顺带同意。
- **只读,不缓存,不落盘**。服务端不保存 transcript 副本,读完即走。
- **绝不把 agent 的原始 schema 透传出去**(R13)。认不出的块归一化或丢弃。
- 每个 task 结束时 `cargo clippy --workspace -- -D warnings` 与全量测试必须干净,
  且**判据取退出码**,不要用管道尾巴(这条栽过两次)。

## File Structure

```
crates/server/src/agent/transcript.rs        消息模型、窗口与游标(与 agent 无关的部分)
crates/server/src/agent/claude_transcript.rs Claude 的 JSONL → 归一化消息
crates/server/tests/agent_transcript.rs
crates/server/tests/fixtures/claude-transcript.jsonl   从真实会话脱敏而来
clients/web/src/chat.ts                      聊天视图的数据层
```

改动:`agent/{mod,store,claude_code,routes}.rs`、`config.rs`、`app.rs`、
`clients/web/src/{main.ts,i18n.ts,style.css,index.html}`、`docs/protocol.md`。

---

### Task 1: 存下 transcript_path,定下消息模型

**Files:**
- Create: `crates/server/src/agent/transcript.rs`
- Modify: `agent/{mod,store,claude_code}.rs`

**Interfaces:**
- Produces: `transcript::{Message, Block, Role}`;`AgentSession.transcript: Option<String>`

- [ ] **Step 1: 写失败的测试**

```rust
#[test] fn transcript_path_is_kept_from_the_hook_payload() {}
#[test] fn a_later_event_without_the_path_does_not_clear_it() {}  // 部分更新不该抹掉已知字段
```

第二条对应已经犯过一次的错:`Update` 里 `None` 是「这次没说」,不是「清空」。

- [ ] **Step 2: 消息模型**

```rust
#[derive(Serialize)] #[serde(rename_all = "kebab-case")]
pub enum Role { User, Assistant, System }

#[derive(Serialize)] #[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Block {
    Text { text: String },
    Thinking { text: String },
    /// `input` 是**给人看的一行**,不是原始 JSON:客户端要展示「它要干什么」,
    /// 而原始入参可以很大;要看细节,终端视图在那儿。
    ToolUse { name: String, input: String },
    ToolResult { ok: bool, text: String },
}

#[derive(Serialize)]
pub struct Message { pub id: String, pub role: Role, pub ts: Option<u64>, pub blocks: Vec<Block> }
```

**不透传未知块类型** —— 认不出的归一化成 `Text` 或直接丢弃。这是 R13 要挡的事。

- [ ] **Step 3: 存路径**

`Update` 加 `transcript: Option<String>`,`claude_code::parse` 从 payload 取
`transcript_path`,`AgentSession` 存下来。只覆盖有值的字段(和 `cwd` / `title` 同规则)。

---

### Task 2: Claude 的 transcript 解析

**Files:**
- Create: `crates/server/src/agent/claude_transcript.rs`
- Create: `crates/server/tests/fixtures/claude-transcript.jsonl`(脱敏样本)
- Modify: `agent/mod.rs`(trait 加 `parse_transcript`)、`claude_code.rs`

- [ ] **Step 1: 造脱敏 fixture**

从本机一份**真实**会话抽 30~50 行,把文本内容替换成占位符,**保留结构**:
`user` / `assistant` 记录、`text` / `thinking` / `tool_use` / `tool_result` 四种块、
以及 `attachment` / `system` / `queue-operation` 这些**要被跳过**的记录类型。

合成数据在这里没有价值 —— 真实风险全在「真文件里到底长什么样」。

- [ ] **Step 2: 写失败的测试**

```rust
#[test] fn four_block_kinds_are_normalized() {}
#[test] fn non_message_records_are_skipped() {}      // attachment / system / queue-operation
#[test] fn unknown_block_kinds_are_dropped_not_leaked() {}   // R13
#[test] fn a_broken_line_does_not_kill_the_window() {}       // 一行坏 JSON 只丢那一行
#[test] fn tool_input_is_flattened_to_one_line() {}
#[test] fn tool_result_error_is_marked() {}
#[test] fn ids_are_stable_across_two_reads() {}      // 增量去重全靠它
```

倒数第二条是增量的地基:同一条消息读两次必须得到同一个 `id`,否则客户端会重复渲染。

- [ ] **Step 3: 实现**

`fn parse_transcript(&self, text: &str) -> Option<Vec<Message>>`。整段文本进,消息出;
**逐行解析,单行失败只跳过那一行** —— 一个坏字节不该让整个窗口变成「读不了」。

`id` 取记录自带的 `uuid` / `message.id`;都没有就用「窗口内序号 + 角色 + 内容哈希」,
保证同样的输入得到同样的 id。

---

### Task 3: 窗口与游标

**Files:**
- Modify: `agent/transcript.rs`
- Create: `crates/server/tests/agent_transcript.rs`

窗口与游标**与 agent 无关**,所以放在共享模块;只有「一行是什么意思」是 adapter 的事。
这是 C3 加 Codex/Grok 时能省下大量重复的地方。

- [ ] **Step 1: 写失败的测试**

```rust
#[test] fn first_read_returns_the_tail_not_the_head() {}      // 第一屏要最近的,不是最早的
#[test] fn window_starts_at_a_line_boundary() {}              // 从末尾回切 N 字节会切在半行上
#[test] fn cursor_resumes_exactly_where_it_left_off() {}
#[test] fn a_shrunk_file_invalidates_the_cursor() {}          // compact / 新会话
#[test] fn message_and_byte_caps_are_enforced() {}
#[test] fn oversized_block_is_truncated_and_marked() {}
```

第二条是这一 task 最容易写错的地方:按字节回切窗口一定会切在半行上,那半行必须丢掉,
否则第一条消息是残的。

- [ ] **Step 2: 实现**

```rust
pub struct Window { pub messages: Vec<Message>, pub cursor: String, pub reset: bool }
```

游标是**服务端发的不透明串**(内部是 `offset:len`)。`len` 用来判定文件是否变小/换过:
变小就 `reset: true` + 整窗重发,客户端整段替换而不是往后追加。

限额(同时写进 protocol.md §10):单次 200 条 / 256 KiB;单块 8 KiB(超出截断并标记);
窗口 = 文件末尾 4 MiB。

---

### Task 4: `GET /api/agents/messages` 与回落

**Files:**
- Modify: `agent/routes.rs`、`app.rs`、`config.rs`

**Interfaces:**
- Produces: `GET /api/agents/messages?agent_id=&agent_session_id=&after=`

- [ ] **Step 1: 写失败的测试**

```rust
#[tokio::test] async fn disabled_by_default_falls_back_to_terminal() {}
#[tokio::test] async fn missing_transcript_path_falls_back() {}
#[tokio::test] async fn unreadable_file_falls_back() {}
#[tokio::test] async fn unknown_agent_session_is_404() {}
#[tokio::test] async fn requires_the_client_bearer() {}
```

- [ ] **Step 2: 回落是带类型的,不是空数组**

```jsonc
{ "source": "transcript", "messages": [...], "cursor": "...", "reset": false }
{ "source": "terminal", "reason": "disabled" | "no-transcript-path" | "unsupported-agent"
                                  | "unreadable" | "parse-failed" }
```

抄的是 orca 的 `source` + `fallbackReason`。他们有个 bug 恰恰是这条没兜住 ——
Windows 上 Codex 会话「聊天界面渲染出一个空 transcript」。**空聊天框比说清原因更糟**:
用户会以为对话真的是空的。

- [ ] **Step 3: 配置与门控**

```toml
[agents]
transcript_enabled = false   # 默认关
```

关着时不是 403 而是 `source: "terminal", reason: "disabled"` —— 这不是错误,是「你没开」,
客户端据此显示一句说明 + 一个「用终端打开」。

- [ ] **Step 4: 读文件是阻塞 IO**

`spawn_blocking`,照抄 `launchers_handler` 与 `agent::routes::list` 的写法。

---

### Task 5: web 端聊天视图(只读)

**Files:**
- Create: `clients/web/src/chat.ts`
- Modify: `clients/web/src/{main.ts,i18n.ts,style.css,index.html}`

- [ ] **Step 1: 数据层 + 测试**

`chat.ts` 只放纯逻辑与 fetch,能被 node:test 直接跑(和 `agents.ts` 同规矩):

```ts
export async function fetchMessages(token, agentId, agentSessionId, after?): Promise<ChatWindow>
/** reset 时整段替换,否则按 id 去重后追加 —— 服务端可能重发边界上的那条 */
export function mergeWindow(prev: Message[], win: ChatWindow): Message[]
```

```
test: reset 整段替换
test: 增量按 id 去重,不重复渲染
test: 路由缺失/报错降级为 source=terminal(T12)
```

- [ ] **Step 2: 视图切换**

会话上绑着 agent 时,终端顶栏出现「聊天 / 终端」切换。**默认仍是终端** —— C1 只读,
不能发消息,把它设成默认会让人以为坏了。

- [ ] **Step 3: 渲染**

四种块各自的样式;`thinking` **默认折叠**(它常常比正文长);`tool-use` 显示工具名 +
那一行入参;`tool-result` 用 `ok` 区分正常/报错。

- [ ] **Step 4: 增量**

收到该会话的 `agent-event` 就带游标拉一次。**不做轮询** —— 没有事件就意味着没有新消息。

- [ ] **Step 5: 回落**

`source: "terminal"` 时按 `reason` 显示一句人话 + 一个「用终端打开」,**绝不画空聊天框**。

---

### Task 6: 文档

- [ ] `docs/protocol.md`:T13 路由表补一行;§10 补三条限额;§1 来源地图补 transcript 模块。
      **`PROTO_VERSION` 不动**,在提交信息里写明为什么不用动(只加 `/api/` 只读路由,T12)。
- [ ] README 双语:说明聊天视图、默认关、以及**它与 desktop 默认监听 `0.0.0.0` 叠加后的
      暴露面** —— 局域网里拿到 token 的人,现在能看终端画面,开了这个之后是整段对话。
- [ ] spec 状态从「草案」改为「C1 已实施」。

## 完成标准

- [ ] `cargo test --workspace` 无 FAILED(**数 `^test result: FAILED` 的行数**,不看汇总)
- [ ] `cargo clippy --workspace -- -D warnings` **退出码为 0**
- [ ] `npx tsc --noEmit` 退出码为 0;`npm test` 无 fail
- [ ] `PROTO_VERSION` 未改动
- [ ] 开关关着时,`/api/agents/messages` 返回 `source: "terminal", reason: "disabled"`,
      且**没有读过任何 transcript 文件**
- [ ] 用一份真实会话手工验证:聊天视图里的消息数、顺序、工具调用与终端里看到的一致
- [ ] 把 transcript 文件改小(模拟 compact)后再拉一次 → `reset: true`,客户端整段替换
