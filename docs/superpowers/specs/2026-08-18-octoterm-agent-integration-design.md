# octoterm agent 集成设计

日期:2026-08-18
状态:草案(范围与三项裁决已由用户确认,细节待评审)

## 目标

让 octoterm 托管的会话里跑着的 coding agent(Claude Code、Codex、pi 等)对客户端
可见、可操作:

1. **看得见** —— 会话列表上能看出「这个会话里跑着 Claude Code,它现在在干活/在等你」;
2. **接得住** —— agent 请求授权或提问时,任何设备上的客户端都能替用户回答;
3. **切得过去** —— 从等待提示直接跳到那个会话的终端画面;
4. **装得上** —— 客户端能扫描本机装了哪些 agent,并一键装/卸 octoterm 的 hook。

这是 README 路线图第 2 条的兑现。

## 非目标

- **不接管非 octoterm 托管的终端**。用户在 iTerm/Windows Terminal 里直接敲起来的
  agent 不在范围内。理由见下一节。
- **不做窗口聚焦**。不上溯 pid 链、不写 AppleScript、不调 SetForegroundWindow、
  不碰 tmux。「切过去」在 octoterm 里就是 `attach` 一个 channel。
- **不做 agent 的用量/配额/成本统计**,不做 transcript 解析与展示。
- **不替用户做决定**。超时、断连、宿主不可用一律回落到「无决定」,让 agent 自己
  在终端里问。唯一例外是 headless 会话(见「降级矩阵」)。

## 立足点:octoterm 是 pty 的主人

参考实现 clawd-on-desk 是一个桌面宠物,它站在所有终端之外,因此必须回答两个很贵
的问题:「这个 agent 跑在哪个终端里」和「怎么把那个终端切到前台」。它为此付出
`hooks/shared-process.js`(1100 行进程树上溯 + 终端进程名白名单 + tmux/HWND 特判)
和 `src/focus.js`(2395 行三平台窗口聚焦胶水)。

octoterm 不需要问这两个问题:

| 问题 | clawd 的答案 | octoterm 托管会话的答案 |
| --- | --- | --- |
| agent 在哪个终端 | 从 hook 自己的 pid 逐层 `ps -o ppid=,comm=` 上溯,命中终端进程名白名单 | spawn 时注入 `OCTOTERM_SESSION_ID`,hook 原样带回来 |
| 怎么切过去 | AppleScript 按 tty 找 iTerm tab / `tmux select-pane` / Windows 常驻 PowerShell + SetForegroundWindow | 客户端 `attach` 那个 session,协议里本来就有 |
| 怎么替用户回答自由文本提问 | **做不到**,官方没有这个接口 | 往 pty 写字节,复用现有数据通道 |

第三行是决定性的:hook 体系只能拦截「结构化的授权/表单请求」,拦不住 agent 在
自己 TUI 里印出来的自由提问。而 octoterm 手里就有那个 pty 的写端。**这是本设计里
唯一 clawd 结构上做不到的能力,也是把范围收在托管会话内的最大理由** —— 收窄范围
反而拿到了更强的能力,同时省掉 3500 行平台胶水。

代价明确:用户在 iTerm 里直接开的 claude 不出现在列表里。接受。想被管就在 octoterm
里开会话。

## 参考实现调研结论

调研对象:clawd-on-desk v0.15.0(23 个 agent)。可复用的与要避开的:

### 值得抄的

- **阻塞式决策的实现方式**:不是长轮询、不是写文件,而是 **HTTP 请求进来后把响应
  挂起不写**,用户点了才 `res.end(...)`。Claude Code 的 `type:http` hook 自己会
  在 socket 上等,最长 `timeout` 秒。在 axum 里这就是 handler 里 `await` 一个
  `oneshot::Receiver`,比 Node 那套挂 `res` 对象还干净。
- **stale 清理的阈值与顺序**(`src/state-stale-cleanup.js`,187 行纯函数):idle 基准
  取 `max(updated_at, acked_at)`;进程已死立即删,优先于一切超时判断;**「超时转
  idle」时不刷新时间戳**,否则永远删不掉。这三条是踩出来的,直接采纳。
- **「宁可不作决定,也不代替用户」**:DND / 宿主不可用 / 弹窗被关,一律断开连接或
  回 204,让 agent 回落到它自己终端里的菜单。
- **所有权标记必须极严**。clawd 的 `isManagedPermissionUrl()` 要求协议、hostname、
  path 正则、端口白名单、无 query/hash/user/password 全部命中才认作「自己写的」,
  就是为了绝不误删用户自建的企业审批端点。同等严格度是底线。

### 要避开的

clawd 有 **4 张平行的表**(运行时 registry / 安装描述符 / 同步修复表 / 卸载表),
没有单一 adapter 接口,`claude-code` 还在同步表外用 if 特判。新增一个 agent 要改
**16 处**。这是支持 23 个 agent 攒出来的形态,不是设计出来的。octoterm 用单一 trait
(见下),新增一个 agent = 一个文件 + 一行注册。

## 四个平面

```
① 客户端控制面   HTTP  /api/agents/*        扫描、装/卸 hook、列 agent 会话、提交回答
② hook 摄入面    HTTP  /hook/:agent/:event  agent 打进来;独立鉴权;决策类请求在此阻塞
③ 状态推送面     WS    ServerMsg::AgentEvent 状态变化实时广播给所有连接
④ 回答注入面     WS    现有数据通道           往 pty 写字节 = 替用户打字
```

① 和 ③ 的分工遵循协议 §12.1 已有的判据:低频、会话无关、socket 起来之前就要用的
走 `/api/`;服务端主动发起的状态通知走控制消息。④ 完全不需要新东西。

## AgentAdapter

对标仓库里现成的 `LauncherProvider`(`crates/server/src/launcher/mod.rs`):多来源、
单一 trait、**失败局部化**(一个 adapter 抛错只让它自己的条目消失,不影响别人)。

```rust
pub trait AgentAdapter: Send + Sync {
    /// 稳定标识,同时是 /hook/:agent/... 的路径段与 UI 分组键
    fn id(&self) -> &'static str;                     // "claude-code"

    /// 本机装没装。目录 + 关键文件 + CLI 三元证据,返回置信度
    fn detect(&self, home: &Path) -> Option<Detected>;

    /// 安装/卸载计划:纯函数,只产出「对哪个文件做什么编辑」,不写盘
    fn plan_install(&self, ctx: &InstallCtx) -> Result<Vec<ConfigEdit>>;
    fn plan_uninstall(&self, ctx: &InstallCtx) -> Result<Vec<ConfigEdit>>;

    /// 把 agent 方言的 hook payload 归一化成统一事件
    fn parse(&self, event: &str, body: &serde_json::Value) -> Result<AgentEvent>;

    /// 把统一决策渲染回 agent 方言的响应体
    fn render(&self, decision: &Decision) -> serde_json::Value;
}
```

### 为什么 plan_install 返回计划而不是直接写盘

三个理由,任一条都够:

1. **可以先给用户看 diff**。装 hook 是往用户的 `~/.claude/settings.json` 里写东西,
   这件事必须能预演。
2. **幂等是免费的**。同一份计划应用两次结果相同,不需要像 clawd 那样写
   `foldManagedStateHooks()` 去按位置折叠重复项 —— 那个函数存在的原因正是旧版
   直接写盘产生了字节相同的重复条目,事后再也无法用命令串谓词区分「留这个删那些」。
3. **单测不碰真实 home**。计划是纯数据,断言计划比断言文件系统副作用容易一个量级。

### 检测规则:三元证据,不靠单一目录

「`~/.claude` 存在」不能证明装了 Claude Code —— 任何写过那个目录的程序都会创建它
(clawd 自己就踩了这个坑,不得不把 `claude-code` 从默认检测里排除)。规则:

| 证据 | 置信度 |
| --- | --- |
| 关键配置文件存在且含非我方内容 | high |
| CLI 可执行文件在 PATH 上 | high |
| 目录存在且含关键子项 | medium |
| 仅目录存在 | low(不展示为「已安装」) |

**不执行 `--version` 之类的命令来做安装判定** —— 扫描是只读操作,不该 spawn 进程。

## 会话模型

内存态,不落盘。server 重启后 hook 的下一个事件就重建了;丢掉的只是历史,而历史
不是本功能的目标。

```rust
struct AgentSession {
    agent_id: &'static str,
    agent_session_id: String,        // agent 自己的 session_id
    octoterm_session: Option<u64>,   // 关联到的托管会话(env 注入得来)
    cwd: Option<String>,
    state: AgentState,
    pending: Option<PendingId>,      // 正在等用户回答的请求
    title: Option<String>,
    updated_at: u64,
    acked_at: u64,
}
```

状态取值(比 clawd 的 11 档少,因为不需要驱动宠物动画):

```
idle | thinking | working | waiting | done | error
```

`waiting` 是本设计里唯一必须精确的状态 —— 客户端的「有事找你」红点全靠它。它有两个
来源,都是权威信号,不靠猜:

- `Notification` 事件且 matcher 命中 `permission_prompt` / `idle_prompt` /
  `agent_needs_input` / `elicitation_dialog`;
- 存在挂起的 `PendingRequest`(见下)。

`octoterm_session` 有值时,状态还多一条硬证据:pty 子进程退出 ⇒ 该 agent 会话必死,
不用等超时。

### 清理

沿用调研得到的阈值,全部可配:

| 常量 | 默认 | 含义 |
| --- | --- | --- |
| `agent_session_stale` | 600 s | 空闲上限 |
| `agent_working_stale` | 300 s | 卡在 working 的保护 |

规则顺序:关联的托管会话已死 → 立即删;超 `working_stale` 且状态是 working/thinking
→ 转 idle(**刷新时间戳**);超 `session_stale` → 删(**不刷新时间戳**)。

## 阻塞式决策链路

```
Claude Code                octoterm-server                    客户端(浏览器/手机)
    │                            │                                    │
    ├─ POST /hook/claude-code/permission ───────────────────────────►  │
    │  (type:http, timeout 600s)  │                                    │
    │                             ├─ 存入 PendingRequest               │
    │                             ├─ 广播 AgentEvent{waiting} ───────► │
    │        ...阻塞在 socket 上... │        ...handler await oneshot... │
    │                             │                                    ├─ 用户点「允许」
    │                             │ ◄─── POST /api/agents/answer ──────┤
    │                             ├─ oneshot.send(Decision)            │
    │ ◄──── 200 + 决策 JSON ───────┤                                    │
    ├─ 继续执行                    ├─ 广播 AgentEvent{working} ───────► │
```

`PendingRequest` 的生命周期由三件事之一终结,**必须都覆盖**:

- 用户回答 → 写响应体;
- 客户端断连/超时 → 「无决定」(对 Claude 是断开连接,对脚本类 hook 是 204);
- **agent 侧断连** → axum 的连接关闭要能取消 handler,否则挂起项会堆到 600 秒。
  clawd 用 `res.on("close", abortHandler)`,Rust 侧靠 handler future 被 drop 时
  清理(`Drop` guard),这一点必须有测试。

### 已实测的线上契约

`PermissionRequest` 请求体字段(实测抓包):

```
cwd, hook_event_name, permission_mode, permission_suggestions,
prompt_id, session_id, tool_input, tool_name, transcript_path
```

注意它**没有 `tool_use_id`**(`PreToolUse` 有)。同样没有任何 pid / tty / 终端字段
—— 这印证了「会话关联只能靠注入的环境变量」这个设计前提。

响应体(实测生效形态,`decision` 是**对象**而不是字符串):

```json
{ "hookSpecificOutput": {
    "hookEventName": "PermissionRequest",
    "decision": { "behavior": "allow", "message": "..." } } }
```

调研阶段拿到的二手资料说 `decision` 是 `"allow"|"deny"|"escalate"|"ask"` 字符串,
**实测不生效**:字符串形态下审批弹窗照常出现,换成对象形态后 TUI 立刻打出
`Allowed by PermissionRequest hook`。以实测为准,adapter 的 `render()` 按对象形态写。

### 降级矩阵

| 场景 | 处理 | 理由 |
| --- | --- | --- |
| 没有任何客户端连着 | 无决定 | agent 会在自己终端里问,那是用户面前的那块屏 |
| 用户没在超时内回答 | 无决定 | 同上 |
| payload 超上限 | 无决定 + 日志 | 见「限额」 |
| agent 的 hook 未启用 | 204 | |

关于 headless:clawd 对 `claude -p` 会话做 auto-deny。**实测表明这一条对我们没有
意义** —— `-p` 模式下 Bash 走沙箱直接硬拒,`PermissionRequest` 根本不触发(两次
用例均未收到该事件)。既然请求到不了我们这里,就没有「代替它决定」的余地。不为
headless 写任何特殊分支。

## 三条回答路径

| 场景 | 路径 | 能力 |
| --- | --- | --- |
| 工具授权(Bash / Edit / …) | `PermissionRequest` 阻塞 hook | allow / deny(+可选追加永久规则) |
| MCP 表单、结构化选择 | `Elicitation` 阻塞 hook | 选项式回答 |
| **agent 自己 TUI 里的自由提问** | **往 pty 写字节** | 任意文本;不需要装任何 hook |

第三条是 octoterm 独占的。它也意味着:**即使一个 agent 完全不支持 hook,只要它跑在
托管会话里,「远程替它打字」这条路依然通** —— 客户端拿到的是会话画面(协议本来就有
`preview`),回答就是往数据通道写字节。hook 体系是增强,不是前提。

## 鉴权与关联

### 托管会话:env 注入,一箭双雕

`pty.rs` 的 spawn 已经在注入 `TERM` / `COLORTERM`,顺手多两个:

```
OCTOTERM_SESSION_ID = <会话 id>
OCTOTERM_HOOK_TOKEN = <hook 专用密钥>
```

Claude Code 的 `type:http` hook 支持 `headers` 与 `allowedEnvVars`(已核实:header 值
支持 `$VAR` / `${VAR}` 插值,且**只有列在 `allowedEnvVars` 里的变量会被解析**,未列出的
被替换成空串),于是写进 settings.json 的是:

```json
{
  "type": "http",
  "url": "http://127.0.0.1:7683/hook/claude-code/permission",
  "headers": {
    "Authorization": "Bearer $OCTOTERM_HOOK_TOKEN",
    "X-Octoterm-Session": "$OCTOTERM_SESSION_ID"
  },
  "allowedEnvVars": ["OCTOTERM_HOOK_TOKEN", "OCTOTERM_SESSION_ID"],
  "timeout": 600
}
```

**鉴权和会话关联一次解决**,而且不需要打包任何外部脚本 —— octoterm-server 自己就是
那个 HTTP 端点。这是相对 clawd(必须随包带 node 运行时 + 一堆 hook js)的第二个
结构性优势,也和「单静态二进制」的定位一致。

### hook 密钥必须独立于 WS token,且持久化

WS token 默认每次启动随机生成(Jupyter 式)。把它写进 `settings.json` 的话,server
一重启,已写入的 hook 就全部失效。因此 hook 密钥单独生成、持久化在 octoterm 自己的
状态文件里,与 WS token 无关。

### 边界

- `/hook/*` **无条件只绑 127.0.0.1**,即使主监听是 `0.0.0.0`。
- 没有合法 `Authorization` 的 hook 请求一律 401,不给细节。

这一条不是形式主义:`fd5a0f9` 刚修过「空白 token 生效 = 空对空一律放行」的鉴权
旁路。`/hook/*` 是同一类洞的新入口,而且它的 payload 里有 `tool_input` —— 命令原文
和文件路径。裸奔(clawd 的做法)在 `--host 0.0.0.0` 的部署下不可接受。

## 安装与卸载

### 所有权标记

写入的每一项都必须能被无歧义地认出是 octoterm 写的,且判定要**极严**:

- HTTP hook:协议 `http:` + hostname `127.0.0.1` + path 严格匹配
  `^/hook/[a-z0-9-]+/[a-z-]+$` + 端口是本机 octoterm 当前/历史监听端口 +
  无 query/fragment/user/password。任何一项不符 → 不是我们的,不碰。
- command hook(若将来需要):命令串必须能被解析成「单条简单调用 + 精确 token」,
  复合命令、重定向、单引号一律 fail closed。

宁可漏删自己的,也绝不误删用户的。

### 写盘规则

1. **默认关闭**。`agents.install_enabled`,默认 `false`。headless / 共享部署可以
   永久关掉这个能力。
2. **写前备份**,保留最近 5 份。
3. **原子写**(tmp + rename)。
4. **卸载必须能把文件恢复到「我们没来过」的状态**:某事件的 hooks 数组被清空后,
   删掉该事件的 key 而不是留一个空数组。

### 与既有硬约束的关系(裁决)

`launcher/mod.rs` 顶部写着「只读是硬约束:octoterm 从不写别人的配置文件」,
`config.rs` 写着「server 自己永不写文件」。本设计破了这两条,裁决是**限定作用域而
不是废除**:

> **发现(discovery)永远只读**;**集成(integration)在用户显式动作下可写**,
> 且受 `agents.install_enabled` 门控、写前备份、提供完整卸载。

两处注释都要改成这个措辞,不留下「代码说 A、行为是 B」的分叉。

## 协议改动与 §12 检查表

### 改动清单

| 改什么 | 类型 | 是否 bump proto |
| --- | --- | --- |
| 新增 `ServerMsg::AgentEvent` | server→client 新消息类型 | **否**(X2:客户端忽略未知 type) |
| 新增 `GET /api/agents` 等只读路由 | 侧信道新路由 | 否(T12) |
| 新增 `POST /api/agents/*` 变更路由 | **与 T10 冲突** | 否,但需修订 T10 |
| 新增 `/hook/*` | 新的第三方入口,不属于现有任何一节 | 否,需新增章节 |

**不新增任何 client→server 控制消息**。回答走 `POST /api/agents/answer`。这是刻意的:
X3 规定新增 client→server 类型是破坏性变更,必须 bump proto 硬切,所有已打开的页面
全断、每个客户端都要重编译。为一个可以用 HTTP 表达的低频请求付这个代价不值。

### R1–R13 应答(摘要,完整版随 PR)

- **R1 复用**:§12.1 的表里没有一行覆盖「服务端主动广播一个与会话弱相关的第三方
  状态」。`session-event` 只描述会话的存在与身份,agent 状态不是会话身份。
- **R2 平面**:`AgentEvent` 低频(人类操作尺度)、结构化、小(见 R10)。合规。
- **R3 兼容性**:X2 + T12,不 bump。T10 需修订 —— 这是文档变更,不是线上变更。
- **R4 无 bulk 字节**:`AgentEvent` 不携带 VT 字节。回答文本走数据通道,不走 JSON。
- **R5 归属**:`/api/agents/answer` 用 `pending_id` 作自然键。
- **R6 重连与重复**:`answer` 幂等 —— 同一 `pending_id` 的第二次提交返回 409,
  不改变已定的决策。重连后客户端用 `GET /api/agents` 重新拉全量,不依赖增量。
- **R7 seq 影响**:「替用户打字」走数据通道写 pty,产生的输出**经由 ring buffer 正常
  计入 seq** —— 和用户自己敲键盘完全同路,没有特殊处理。
- **R8 状态机**:`/api/` 需已鉴权;`/hook/*` 在连接状态机之外,自带鉴权。
- **R9 失败**:HTTP 状态码,不发 `error` 控制消息(T11)。
- **R10 限额**:`AgentEvent` ≤ 4 KiB;hook payload ≤ 64 KiB(状态类)/ 512 KiB
  (决策类,`tool_input` 可能含长命令);超限拒收并记日志。加进 §10。
- **R11 产物**:同一 PR 要动 `messages.rs`、两个 fixture、`clients/web/src/*`、
  `crates/server/tests/`、`docs/protocol.md`(新规则 ID)。
- **R12 命名**:`agent-event`、`agent_session_id`、状态值 kebab-case。
- **R13 客户端中立**:`AgentEvent` 只描述「哪个 agent、哪个会话、什么状态、有没有在
  等人」,不含任何窗口/标签/面板语义,也不要求客户端理解服务端数据结构。合规。

## 风险

### 🔴 连接失败会被 Claude Code 当成 deny

**已实测复现,结论:clawd 的说法不成立,官方文档为准。风险解除。**

复现环境:本机 Claude Code,用 `--settings` 追加一个 http hook(不改用户全局配置),
交互式会话由 pty 驱动,工具选用需要审批的 `touch ./marker`。

| 用例 | hook 事件 | 端点 | 结果 |
| --- | --- | --- | --- |
| A1 | PreToolUse | 活,返回 `{}` | 工具执行 ✓(测试台可用) |
| A2 | PreToolUse | 活,返回 deny | **工具被拦**,`permission_denials` 回填,拒绝理由透传 ✓(测试台能拦) |
| A3 | PreToolUse | **死端口** | **工具照常执行**,无 denial |
| A4 | PreToolUse | 活,返回 500 | 工具照常执行 |
| A5 | PreToolUse | 活,挂起至超时 | 工具照常执行 |
| C1 | PermissionRequest | 活,返回 allow | 工具执行,TUI 打出 `Allowed by PermissionRequest hook` ✓ |
| C2 | PermissionRequest | **死端口** | **正常审批弹窗照常出现**,等用户选择,无拒绝 |
| C3 | PermissionRequest | 活,挂起至超时 | 同 C2,超时后回落到正常审批弹窗 |

A2 是关键的阴性对照:它证明测试台确实能拦住工具,因此 A3/C2 的「没拦住」不是测试
台失灵。

**结论**:连接失败、非 2xx、超时三种失败面,对 `PreToolUse` 与 `PermissionRequest`
一律是 non-blocking —— 落回 Claude 自己的审批流程,把选择权交还给终端前的用户。
这正是本设计想要的降级语义,**不需要任何兜底机制**。

因此:

- **对策 1(hook 生命周期跟随 server)取消**。hook 可以常驻,server 起停不影响
  Claude 可用性。这也让安装语义简单得多 —— 装一次就行。
- 保留一条弱化的自检:`GET /api/agents` 报告「已安装但 server 未运行」,理由不再是
  「会被误拒」,而是「远程接管此刻不可用,提示会落在本机终端上」。

octoterm 不是。用户随手起停 server,而 hook 一旦写进 `settings.json` 就一直在。
**hook 装着、server 没跑 = 所有 Claude 授权请求被静默拒绝**,且极难归因。

对策(全上):

1. **`PermissionRequest` hook 的生命周期跟随 server**:desktop 退出时摘掉,启动时
   装回。状态类 hook(`async:true`,失败无副作用)不受此限。
2. 安装 UI 必须说明这个副作用,不能只显示「已安装」。
3. `GET /api/agents` 返回自检项:「已安装但 server 未运行」要能被检出并提示。

### 🟠 Codex 侧资料互相矛盾,且装完不自动生效

官方 `docs/config.md` 只提到 lifecycle hooks 与 `allow_managed_hooks_only`;社区
资料一说 11 个事件、一说只有 `PreToolUse`/`PostToolUse` 且只认 `deny`、不支持 http
type。而 clawd 实际写的是 `{"hooks":{…}}` 包裹的 6 事件版本 + `config.toml` 里
`[features] hooks = true`。

更麻烦的是 **Codex 用 `trusted_hash` 门控**:装完 hook 后用户必须进 TUI 敲 `/hooks`
review 才生效,这是无法自动化的人工步骤。

裁决:**Codex 放第二期**,进场前先做实测验证。第一期只做 Claude Code。

### 🟡 体积

release profile 是 `opt-level='z' + lto + strip`,「极小常驻」是 README 的卖点。
第一期新增依赖上限:**`toml_edit` 一个**(第二期改 Codex `config.toml` 时才需要)。
JSONC 解析仓库里已有(`launcher/jsonc.rs`)。明确不引入 `sysinfo` 之类 —— 那是被
排除的「非托管终端」范围才需要的东西。

## 分期

| 期 | 内容 | 量级 |
| --- | --- | --- |
| **P1** | `AgentAdapter` + registry;Claude Code adapter;`/api/agents` 扫描/装卸/自检;`/hook/claude-code/:event` 摄入 + 阻塞决策;会话表 + 清理;`AgentEvent` 广播;env 注入;web 端面板与回答 UI | Rust ~2500–3500 行,TS ~600 行,无 proto bump |
| **P2** | Codex adapter(先实测);多 agent 的 UI 分组 | ~800 行/agent |
| **P3(暂不做)** | 非托管终端:pid 链上溯 + 三平台窗口聚焦 | ~3500 行平台胶水 |

## 测试

- **adapter 层**:`plan_install` 是纯函数 —— 给定一份 settings.json 文本,断言产出的
  编辑计划;重复应用两次断言结果相同(幂等);断言不碰非我方条目。
- **所有权判定**:一组「长得像但不是我们的」URL/命令串,断言全部不被认领。
- **阻塞链路**:三条终结路径各一个集成测试,重点是 **agent 侧断连要能取消挂起项**。
- **降级矩阵**:每一行一个用例。
- **清理**:阈值边界 + 「超时转 idle 不刷新时间戳」的回归测试。

## 明确不做

- 不做 agent 的用量、配额、成本展示。
- 不做 transcript 解析、对话历史浏览 —— 那是 agent 自己的 UI 的事。
- 不做窗口聚焦、不做非托管终端发现(见「非目标」)。
- 不做自动决策/策略引擎。octoterm 是通道,不是审批系统。
