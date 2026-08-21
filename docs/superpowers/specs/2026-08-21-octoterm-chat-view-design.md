# octoterm 聊天视图设计

日期:2026-08-21
状态:草案(待评审)

## 目标

托管会话里跑着 coding agent 时,客户端可以用**聊天视图**打开它:结构化的消息流
(用户说了什么、它想了什么、调了哪个工具、结果如何),在手机上像用 chatbox 一样
看进度、回消息、拍板授权 —— 而不是盯着一块 80×24 的 VT 画面。

终端视图不会被取代。聊天视图是**同一个会话的另一种打开方式**,两者随时切换。

## 非目标

- **不重做 agent 的 UI**。不追求把每种内容块都渲染得和它自己的 TUI 一样好。
- **不解析终端画面**。理由见下一节。
- **不写 agent 的任何数据**。transcript 只读。
- **不做历史检索/搜索/导出**。那是别的产品。
- **不支持 Grok**(P1)。理由见「各家的来源」。

## 立论:三条腿,以及它们各自的来源

调研对象是 stably/orca —— 它做到了同一件事,而且它自己的 CLI 文档把机制写得很清楚。
结论是这件事需要**三样东西**,缺一不可,而且它们**不能互相替代**:

| 要素 | 来源 | 为什么不能用别的顶替 |
| --- | --- | --- |
| **内容** | agent 自己写的 transcript(JSONL) | 终端里只有 VT 字节,没有「消息」这个概念 |
| **输入** | 往 pty 写字节 | agent 没有对外的输入通道;写 pty 与人敲键盘不可区分 |
| **时序 + 身份** | hook | 只有它能**同步拦截**,也只有它能证明「这个 transcript 属于哪个会话」 |

octoterm 已经有第二条(pty 是我们开的)和第三条(hook 摄入面已在跑),**缺的只有第一条**。
更准确地说:`transcript_path` 我们**每次都收到,但直接扔掉了** —— `claude_code::parse`
只取了 `cwd` / `tool_name` / `session_title`。

### 为什么不是解析终端画面

VT 流里没有消息边界。TUI 会重绘、会滚动、会切 alt-screen、会用光标定位覆盖既有内容 ——
从渲染结果反推「刚才那是第几条助手消息」是没法做对的事,只能做到某种程度的像。
而 transcript 里本来就是**未经渲染的 API 消息对象**。

实测(本机一份 5258 行的真实 transcript):

```
记录类型: assistant 1640, user 999, system 525, attachment 156 …
内容块:   tool_result 726, tool_use 715, text 499, thinking 429
```

要渲染聊天需要的一切都在里面,一个字节的 VT 都不用碰。

### 为什么 transcript 不能取代 hook

这是最容易搞反的一点。transcript 是**只读的、事后的**日志:

- **它没法拦截**。授权请求是一个同步决策点,读文件永远回答不了它。
- **它分不清「在跑」和「在等你」**。活着的会话里,一个没有配对 `tool_result` 的
  `tool_use` 两种情况都可能。实测那份已结束的会话里 706 个 `tool_use` **全部**有配对,
  正说明这个信号只在「还没结束」时有值,而它恰恰不告诉你在等什么。
- **它不知道自己属于谁**。哪个文件对应哪个 pty 会话,要么 hook 告诉你,要么靠猜。

所以分工是:**内容交给 transcript,拦截与推送留给 hook**。hook 现在承担的「用
`tool_name` 当摘要」这类活可以还给 transcript。

## 消息模型

对外的形状必须是**客户端中立**的(protocol.md R13):客户端不该知道 Claude 的
`content block` 长什么样,更不该为 Codex 再学一套。adapter 负责把各家方言归一化。

```
Message {
  id: str,              # 稳定,用于去重与增量
  role: "user" | "assistant" | "system",
  ts: u64?,             # unix 秒,拿不到就没有
  blocks: [Block],
}

Block =
  | { kind: "text",        text: str }
  | { kind: "thinking",    text: str }
  | { kind: "tool-use",    name: str, input: str }   # input 是**给人看的一行**,不是原始 JSON
  | { kind: "tool-result", ok: bool, text: str }
```

三个刻意的取舍:

1. **`tool-use.input` 是压平的一行**,不是原始 JSON。客户端要展示的是「它要干什么」,
   而原始入参可以很大;真要看细节,终端视图在那儿。
2. **`thinking` 单独成块而不是丢掉**。它是这类 agent 最有信息量的部分之一,客户端
   自己决定折不折叠。
3. **不透传未知块类型**。认不出的块归一化成 `text` 或直接丢弃,绝不把 agent 的内部
   结构漏到线上 —— 那正是 R13 要挡的事。

## 各家的 transcript 从哪来

| agent | 位置 | 怎么定位 | 状态 |
| --- | --- | --- | --- |
| Claude Code | `~/.claude/projects/<cwd-slug>/<session>.jsonl` | **hook payload 里直接给 `transcript_path`**(已实测,每个事件都有) | ✅ P1 |
| Codex | `~/.codex/sessions/<Y>/<M>/<D>/rollout-<ts>-<uuid>.jsonl` | hook payload **不给**路径(codex 二进制里没有这类字段),但文件名末尾的 uuid **就是** `session_meta.id`,可由 `session_id` 确定性推导(已实测) | ✅ P1(需实测 hook payload 是否含 session_id) |
| Grok | 未找到 | `~/.grok/memtrace/*.jsonl` 的字段是 `kind/pid/ts_ms/version`,是内存/遥测追踪,**不是对话记录**;会话内容在哪没查到 | ❌ 不进 P1 |

**Grok 的处理方式是诚实地不支持**:检测到是 Grok 就只给终端视图,并说明原因。
半吊子的聊天视图比没有更糟 —— 它会让人以为自己看到了全部。

## 主备回落(抄 orca 的一条设计)

orca 的 `worker-read` 返回 `source: "transcript" | "terminal"`,证不出 transcript 时
退回**有界的终端输出**,并附一个带类型的 `fallbackReason`。他们有个 bug 恰恰是这条
没兜住:Windows 上 host-runtime 的 Codex 会话「聊天界面渲染出一个空 transcript」。

octoterm 照抄这个思路,但更进一步:**回落目标就是我们已有的终端视图**。

```
GET /api/agents/messages?... →
  { source: "transcript", messages: [...], cursor: "..." }
  { source: "terminal", reason: "no-transcript-path" | "unreadable" | "unsupported-agent" | "parse-failed" }
```

聊天视图拿到 `source: "terminal"` 时,不画空聊天框,而是显示原因 + 一个「用终端打开」。

## 传输

- **历史走 HTTP**:`GET /api/agents/messages`,带游标、有窗口上界。**不进控制通道** ——
  协议 R4 明令不许在那里走大块数据,而一段对话可以是几 MB。
- **增量靠现有的 `agent-event` 触发拉取**。广播继续只说「有事了」,客户端收到就带着
  游标去拉新的那一段。这和挂起详情(`/api/agents/pending`)、会话全量(A5)是同一个路子,
  不需要新的推送机制,也不需要动 proto。
- **游标是服务端发的不透明串**(内部是字节偏移)。文件变小/换了(compact、新会话)时
  服务端判定游标失效,返回一整个窗口 + 新游标,客户端整段替换。

## 发消息:写 pty,但要有一道闸

发送本身很简单 —— 往那个会话的数据通道写字节 + `\r`,和人敲键盘不可区分,octoterm
现在就能做。orca 的做法也是这个(`terminal send --text ... --enter`,它的文档明确写着
"not a separate control channel")。

**难的是什么时候能发。** orca 为此专门有一个 `terminal wait --for tui-idle`,文档说它
"essential before sending follow-up input to TUI agents"。

octoterm 手上有三个 orca 只能靠启发式去猜的信号:

| 信号 | 来源 | 强度 |
| --- | --- | --- |
| 回合结束 / 正在等人 | hook 的 `Stop` / `Notification` | 推送、权威 |
| 有没有挂着授权菜单 | 我们自己的 pending 表 | **确定性** —— 那个菜单就是我们挡下来的 |
| 屏幕当前长什么样 | `SessionGrid`(为 resync 维护的服务端权威 grid) | 看渲染结果,不猜字节流 |

于是闸的规则可以是硬的,不是启发式:

| 状态 | 自由文本 | 理由 |
| --- | --- | --- |
| `waiting` **且有 pending** | **禁止** | TUI 在等按键。发一句话过去,**第一个字符会被当成菜单选项** |
| `working` / `thinking` | 允许 | Claude Code 自带输入队列(实测 transcript 里 `queue-operation`: enqueue 305 / dequeue 252),忙时敲的字不会丢 |
| `idle` | 允许 | |

> ⚠️ 这条闸现在**是缺的**。已有的「去这个会话」按钮会把用户直接丢进终端,如果那时
> 授权菜单还挂着,他打的第一个字就变成了选择。这是个现存的坑,不是新功能带来的。

## 隐私

这是本设计里风险最大的一处,必须写在明面上:

**transcript 是整个会话的全部内容** —— 代码、路径、命令、模型的思考过程。现在网络上
跑的最多是 `tool_input`(一条命令),聊天视图会把暴露面**抬高一个数量级**。

三条约束:

1. **独立开关**,`agents.transcript_enabled`,**默认关**。装 hook 是一个决定,把整段
   对话送上网是另一个决定,不能靠前者顺带同意。
2. **和 desktop 默认监听 `0.0.0.0` 叠在一起要重新评估**。局域网里拿到 token 的人,
   现在能拿到的是终端画面,开了这个之后是全部对话。文档必须把这两件事放在一起说。
3. **只读、不缓存、不落盘**。服务端不保存 transcript 的副本,读完即走。

## 限额

| 项 | 值 | 理由 |
| --- | --- | --- |
| 单次响应消息数 | 200 | 一屏历史足够,再多靠游标翻 |
| 单次响应字节 | 256 KiB | 与 hook body 上限同量级 |
| 单个块文本 | 8 KiB(超出截断并标记) | 一次 `cat` 的输出可以是几 MB |
| 读取窗口 | 文件末尾 4 MiB | 不为了第一屏去读一个 100 MB 的文件 |

## 分期

| 期 | 内容 |
| --- | --- |
| **C1** | 存下 `transcript_path`;Claude adapter 的解析;`GET /api/agents/messages` + 游标 + 回落;web 端聊天视图(只读) |
| **C2** | 发消息 + 那道闸(含补掉「去这个会话」的现存坑) |
| **C3** | Codex adapter 的解析(先实测它的 hook payload 有没有 `session_id`) |
| **不做** | Grok(来源未知);检索/导出 |

## 明确不做

- 不解析终端画面来重建消息。
- 不写、不改、不缓存 agent 的 transcript。
- 不把 agent 的原始 schema 透传到线上(R13)。
- 不在 transcript 读不到时假装有聊天界面 —— 说清原因,回落到终端。
