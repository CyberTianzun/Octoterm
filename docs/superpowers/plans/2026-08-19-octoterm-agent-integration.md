# octoterm agent 集成实施计划(P1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 octoterm 托管的会话里跑着的 Claude Code 对客户端可见、可接管:扫描本机 agent、装/卸 hook、实时看到会话状态、在任何设备上回答它的授权请求与提问。

**Architecture:** server 里新增一个 `agent` 模块。对外是三个面:`/api/agents/*`(客户端控制,变更类)、`/hook/{agent}/{event}`(agent 打进来,独立鉴权,决策类请求在此阻塞)、`ServerMsg::AgentEvent`(状态广播)。「替用户打字」不新增任何东西,直接复用现有数据通道往 pty 写字节。多 agent 靠单一 `AgentAdapter` trait 横向扩展,形态照抄现成的 `LauncherProvider`。

**Tech Stack:** Rust 2024 / axum 0.8 / tokio / serde_json / uuid —— **P1 不新增任何依赖**。

设计依据:`docs/superpowers/specs/2026-08-18-octoterm-agent-integration-design.md`

## Global Constraints

- **P1 只做 Claude Code,只管托管会话**。不写 pid 链上溯、不写窗口聚焦、不做 Codex(见 spec「非目标」与「风险」)。
- **不 bump `PROTO_VERSION`**。只允许新增 server→client 消息类型(X2)与新增 HTTP 路由(T12);**绝不新增 client→server 控制消息**(X3 是破坏性变更)。
- **`/hook/*` 无条件只绑 `127.0.0.1`**,即使主监听是 `0.0.0.0`。
- **写别人的配置文件必须过三道**:`agents.install_enabled` 开关(默认 `false`)、写前备份、卸载能还原。所有权判定宁可漏删自己的,绝不误删用户的。
- 每个 task 结束时 `cargo clippy --workspace -- -D warnings` 必须干净,`cargo test --workspace` 必须绿。
- 全部注释与用户可见文案用简体中文,与现有代码一致。
- **axum 0.8 的路径参数是 `{name}` 不是 `:name`**(0.7 → 0.8 的破坏性变更),写路由时别照抄旧写法。

## File Structure

```
crates/server/src/agent/mod.rs           AgentAdapter trait、注册表、共享类型
crates/server/src/agent/detect.rs        本机检测(三元证据)
crates/server/src/agent/edit.rs          配置编辑计划 + 所有权判定(纯函数)
crates/server/src/agent/apply.rs         落盘执行:门控、备份、原子写
crates/server/src/agent/claude_code.rs   Claude Code adapter
crates/server/src/agent/store.rs         会话表、PendingRequest、stale 清理
crates/server/src/agent/routes.rs        /api/agents/* 与 /hook/{agent}/{event}
crates/server/tests/agent_detect.rs
crates/server/tests/agent_edit.rs
crates/server/tests/agent_install.rs
crates/server/tests/agent_hook.rs
crates/server/tests/agent_pending.rs
clients/web/src/agents.ts                agent 面板与回答 UI
```

改动的既有文件:`crates/protocol/src/messages.rs`、两份 fixtures、`crates/server/src/{app,config,lib}.rs`、`crates/server/src/session/pty.rs`、`crates/server/src/launcher/mod.rs`(注释措辞)、`docs/protocol.md`、`README.md` / `README-cnzh.md`、`clients/web/src/main.ts`。

---

### Task 1: agent 模块骨架、trait、检测与只读扫描路由

**Files:**
- Create: `crates/server/src/agent/mod.rs`、`agent/detect.rs`、`agent/claude_code.rs`
- Create: `crates/server/tests/agent_detect.rs`
- Modify: `crates/server/src/lib.rs`(加 `pub mod agent;`)、`crates/server/src/app.rs`(挂路由)

**Interfaces:**
- Produces:
  - `octoterm_server::agent::{AgentAdapter, Detected, Confidence, AgentStatus, registry}`
  - `GET /api/agents` → `{ "agents": [AgentStatus] }`

- [x] **Step 1: 写失败的测试**

`crates/server/tests/agent_detect.rs` —— 检测必须是三元证据,且**不能被空目录骗过**:

```rust
use octoterm_server::agent::{detect, Confidence};
use std::fs;

#[test]
fn 空目录不算安装() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join(".claude")).unwrap();
    let d = detect::claude_code(home.path());
    assert!(!d.installed, "只有一个空目录不足以判定已安装");
}

#[test]
fn 有配置文件且含非我方内容算高置信() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".claude");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
    let d = detect::claude_code(home.path());
    assert!(d.installed);
    assert_eq!(d.confidence, Confidence::High);
}

#[test]
fn 只有我方写入的配置不算用户装过() {
    // settings.json 里只有 hooks,且 hooks 全是我们的 → 证明不了 Claude Code 存在
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".claude");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("settings.json"),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"http","url":"http://127.0.0.1:7683/hook/claude-code/stop"}]}]}}"#,
    ).unwrap();
    let d = detect::claude_code(home.path());
    assert!(!d.installed);
}
```

第三个用例是 clawd 踩过的坑:它自己会创建 `~/.claude/`,导致「目录存在」变成自证。

- [x] **Step 2: 定义 trait 与共享类型**

`crates/server/src/agent/mod.rs`:

```rust
//! agent 集成。扩展点只有一个:[`AgentAdapter`]。
//!
//! 形态照抄 `launcher` 模块 —— 多来源、单一 trait、**失败局部化**:某个 adapter
//! 抛错只让它自己的条目消失并留一条日志,不影响其他 adapter,更不影响终端本身。
//!
//! 与 launcher 的关键区别:launcher **只读**扫描别人的配置,agent 集成在用户显式
//! 动作下**会写**别人的配置。这条例外的边界写在 spec 里,并由 `apply.rs` 的三道
//! 关卡(开关 / 备份 / 可还原)守住。

use serde::Serialize;
use std::path::{Path, PathBuf};

pub mod apply;
pub mod claude_code;
pub mod detect;
pub mod edit;
pub mod routes;
pub mod store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize)]
pub struct Detected {
    pub installed: bool,
    pub confidence: Confidence,
    /// 机器可读的判定依据:"config-file" / "cli-path" / "parent-dir" / "not-found"
    pub reason: &'static str,
    /// 给人看的一句话。UI 直接显示,不解析。
    pub detail: String,
    pub config_path: Option<PathBuf>,
}

/// 我方集成在不在。`Foreign` 表示那个位置有东西但不是我们写的 —— 必须能和
/// `NotInstalled` 区分开,否则「装一下」会变成「覆盖用户的东西」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Integration {
    NotInstalled,
    Installed,
    /// 已装,但 URL 指向的端口不是当前监听端口 —— 远程接管此刻是失效的
    StalePort,
    Foreign,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub detected: Detected,
    pub integration: Integration,
}

pub trait AgentAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;

    fn detect(&self, home: &Path) -> Detected;

    /// 只产出「要对哪个文件做什么编辑」,**不写盘**。见 spec:可预演 diff、
    /// 幂等免费、单测不碰真实 home。
    fn plan_install(&self, ctx: &edit::InstallCtx) -> anyhow::Result<Vec<edit::ConfigEdit>>;
    fn plan_uninstall(&self, ctx: &edit::InstallCtx) -> anyhow::Result<Vec<edit::ConfigEdit>>;

    /// agent 方言的 hook payload → 统一事件
    fn parse(&self, event: &str, body: &serde_json::Value) -> anyhow::Result<store::AgentEvent>;

    /// 统一决策 → agent 方言的响应体
    fn render(&self, decision: &store::Decision) -> serde_json::Value;
}

pub fn registry() -> Vec<Box<dyn AgentAdapter>> {
    vec![Box::new(claude_code::ClaudeCode)]
}

pub fn find(id: &str) -> Option<Box<dyn AgentAdapter>> {
    registry().into_iter().find(|a| a.id() == id)
}
```

- [x] **Step 3: 实现检测**

`agent/detect.rs` 按 spec 的三元证据表实现。要点:

- `~/.claude/settings.json` 存在且 **JSON 里有除 `hooks` 外的任何 key**,或 `hooks` 里有非我方条目 → `High` / `"config-file"`;
- 否则 `claude` 可执行文件在 `PATH` 上 → `High` / `"cli-path"`;
- 否则 `~/.claude/` 存在且含 `settings.json` 以外的关键子项 → `Medium` / `"parent-dir"`;
- 否则 `Low` / `"not-found"`,`installed = false`。

**不执行 `claude --version`** —— 扫描是只读操作,不 spawn 进程。

- [x] **Step 4: 挂只读路由**

`app.rs` 里加 `.route("/api/agents", get(crate::agent::routes::list))`,鉴权复用现成的 `bearer_ok`。扫描是阻塞 IO,**必须 `spawn_blocking`**,照抄 `launchers_handler` 的写法。

- [x] **Step 5: 验证**

`cargo test -p octoterm-server --test agent_detect` 全绿;手工 `curl -H "Authorization: Bearer <token>" localhost:7683/api/agents` 能看到本机结果。

---

### Task 2: 配置编辑计划与所有权判定(纯函数)

**Files:**
- Create: `crates/server/src/agent/edit.rs`
- Create: `crates/server/tests/agent_edit.rs`
- Modify: `crates/server/src/agent/claude_code.rs`(实现 `plan_install` / `plan_uninstall`)

**Interfaces:**
- Produces:
  - `edit::{InstallCtx, ConfigEdit, EditOp, apply_to_json, is_ours}`

这一 task 是整个功能里**最需要测试密度**的地方:它是唯一会改用户文件的逻辑。

- [x] **Step 1: 写失败的测试**

`crates/server/tests/agent_edit.rs`:

```rust
use octoterm_server::agent::edit::{apply_to_json, is_ours, InstallCtx};
use octoterm_server::agent::{find, AgentAdapter};
use serde_json::json;

fn ctx() -> InstallCtx { InstallCtx { home: "/home/u".into(), port: 7683 } }

fn plan_applied(mut doc: serde_json::Value) -> serde_json::Value {
    let a = find("claude-code").unwrap();
    for e in a.plan_install(&ctx()).unwrap() {
        apply_to_json(&mut doc, &e.op).unwrap();
    }
    doc
}

#[test]
fn 幂等_应用两次结果相同() {
    let once = plan_applied(json!({}));
    let twice = plan_applied(once.clone());
    assert_eq!(once, twice, "同一份计划应用两次必须收敛");
}

#[test]
fn 不碰用户自己的 hook() {
    let user = json!({"hooks":{"Stop":[{"matcher":"","hooks":[
        {"type":"command","command":"my-own-script.sh"}]}]}});
    let out = plan_applied(user.clone());
    let stop = &out["hooks"]["Stop"];
    let kept = stop.as_array().unwrap().iter().any(|g|
        g["hooks"][0]["command"] == "my-own-script.sh");
    assert!(kept, "用户自己的 hook 必须原样保留");
}

#[test]
fn 卸载后回到原样() {
    let user = json!({"model":"opus","hooks":{"Stop":[{"matcher":"","hooks":[
        {"type":"command","command":"my-own-script.sh"}]}]}});
    let mut doc = plan_applied(user.clone());
    let a = find("claude-code").unwrap();
    for e in a.plan_uninstall(&ctx()).unwrap() {
        apply_to_json(&mut doc, &e.op).unwrap();
    }
    assert_eq!(doc, user, "卸载必须能还原到我们没来过的状态");
}

#[test]
fn 卸载清空后删掉事件键而不是留空数组() {
    let mut doc = plan_applied(json!({}));
    let a = find("claude-code").unwrap();
    for e in a.plan_uninstall(&ctx()).unwrap() {
        apply_to_json(&mut doc, &e.op).unwrap();
    }
    assert_eq!(doc, json!({}), "不能留下 {{\"hooks\":{{\"Stop\":[]}}}} 这种残渣");
}

/// 所有权判定必须**极严**:长得像但不是我们的,一律不认领。
#[test]
fn 长得像的一律不认领() {
    let mine = json!({"type":"http","url":"http://127.0.0.1:7683/hook/claude-code/stop"});
    assert!(is_ours(&mine, 7683));

    for bad in [
        json!({"type":"http","url":"https://127.0.0.1:7683/hook/claude-code/stop"}), // 协议不同
        json!({"type":"http","url":"http://127.0.0.2:7683/hook/claude-code/stop"}),  // 主机不同
        json!({"type":"http","url":"http://127.0.0.1:9999/hook/claude-code/stop"}),  // 端口不同
        json!({"type":"http","url":"http://127.0.0.1:7683/hook/claude-code/stop?x=1"}), // 带 query
        json!({"type":"http","url":"http://u:p@127.0.0.1:7683/hook/claude-code/stop"}), // 带凭据
        json!({"type":"http","url":"http://127.0.0.1:7683/permission"}),             // 别家的端点
        json!({"type":"command","command":"curl 127.0.0.1:7683/hook/claude-code/stop"}), // 不是 http 型
    ] {
        assert!(!is_ours(&bad, 7683), "不该认领: {bad}");
    }
}
```

- [x] **Step 2: 实现**

```rust
pub struct InstallCtx { pub home: PathBuf, pub port: u16 }

pub struct ConfigEdit { pub path: PathBuf, pub op: EditOp }

pub enum EditOp {
    /// 保证该事件下**恰好有一条**我方 hook,字段与 spec 一致;其余条目原样不动
    EnsureHook { event: String, spec: serde_json::Value },
    /// 删掉该事件下所有我方 hook;数组空了就删掉事件键
    RemoveOurs { event: String },
}
```

`apply_to_json` 是纯函数:`&mut serde_json::Value` 进,原地改。`is_ours` 只认 `type == "http"` 且 URL 逐段匹配(协议 / 主机 / 端口 / path 正则 `^/hook/[a-z0-9-]+/[a-z-]+$` / 无 query / 无 fragment / 无凭据)。

**不引入 `url` crate**:手写解析,或用已有依赖。P1 的约束是不加依赖。

- [x] **Step 3: 写 Claude Code 的 hook 规格**

遥测类(`async: true`, `timeout: 5`):`SessionStart` `SessionEnd` `UserPromptSubmit` `PreToolUse` `PostToolUse` `Stop` `Notification`
阻塞类(`timeout: 600`,不带 `async`):`PermissionRequest`

单条 hook 长这样:

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

> ⚠️ **待实测**:`async: true` 与 header 的环境变量插值是否共存良好,官方文档没写。实现这一步时先手工验证遥测类事件能带着 `Authorization` 头打进来;如果不行,遥测类去掉 `async` 改用 `timeout: 5`(server 是即答的,代价可接受)。

- [x] **Step 4: 验证**

`cargo test -p octoterm-server --test agent_edit` 全绿。幂等与「卸载后回到原样」两条是本 task 的验收线。

---


---

## 实施记录:Task 1 / Task 2(2026-08-19,已完成)

21 个用例全绿(`agent_detect` 8 + `agent_edit` 13),`cargo clippy --workspace -- -D warnings`
干净(CI 用的就是这条口径,不带 `--all-targets`)。手工验证:真实 `~/.claude` 上
`GET /api/agents` 返回 `installed/high/config-file`,并正确报出本机 clawd-on-desk 的
冲突 hook。

**与计划的偏差**(都是实现时发现计划写得不对,不是妥协):

1. **测试函数名用 ASCII**。计划里我写的是中文函数名,但仓库现有测试全是 ASCII 命名,
   跟着仓库走,中文放文档注释。
2. **`detect(&DetectEnv)` 而不是 `detect(&Path)`**。PATH 必须能注入 —— 否则单测会读到
   开发机上真实存在的 `claude`,把「没装」的用例全部染成「装了」。这是写第一个用例时
   立刻暴露的。
3. **trait 里暂时没有 `parse` / `render`**。它们要到 Task 5/6 才有实现,现在放进去只能
   写 `todo!()`。等用得上时再加,不留占位桩。
4. **`Integration::Foreign` 换成 `AgentStatus.conflicts: Vec<String>`**。Claude Code 的
   hooks 是**列表**,不存在「被别人占住的槽位」,`Foreign` 这个变体在这里根本无法产生。
   但真正有意义的互操作问题是另一回事:同一事件上挂着**别人的阻塞式 hook**(本机装了
   clawd 就会),这不该删也删不得,但必须报出来。改成一个人类可读的冲突列表。
5. **`apply_to_json` 不需要端口**。删除时按「我方形状、**任意**端口」匹配 —— 用户改过
   监听端口之后,旧端口的条目仍然是我们的垃圾,卸载必须能清掉。`is_ours` 才看端口,
   那是为了把「装了却端口对不上」这种**没有任何外部症状**的失效状态报出来。
6. **开了 `serde_json/preserve_order`**。计划说 P1 不加依赖 —— 这是既有依赖上的 feature,
   `indexmap` 早已在依赖树里,不引入新的编译单元。理由:我们要重写**用户的**
   `settings.json`,默认的 `BTreeMap` 会把人家的 `env` / `permissions` / `model` 按字典序
   重排整个文件。已加回归测试 `preserves_user_key_order`。
7. **`AppState` 多了 `listen_port`**。装 hook 要把端口写进 URL,判定 `StalePort` 也要它。
   取自 `listener.local_addr()` 而不是配置值 —— 配 `:0` 时两者不同。三处构造点同步更新。

**新增的测试资产**:`crates/server/tests/fixtures/claude-settings-with-other-vendor.json`
—— 从本机真实配置脱敏而来(15 个事件、14 条 command hook + 1 条指向别家端口的阻塞式
http hook,外加 env/permissions/model)。合成用例覆盖不到「多事件 × 多组 × 混合类型」
同时出现时的组清理逻辑,而那正是最容易把别人的条目一起带走的地方。

**留给后续 task 的已知缺口**:`$CLAUDE_CONFIG_DIR` 没处理(Claude Code 支持用它换掉
`~/.claude`),代码里已标注。

### Task 3(改版): 安装 = 写我们自己的目录,不碰用户的文件

> **本 task 于 2026-08-19 依据四条实测结论重写**,原方案(编辑 `~/.claude/settings.json`)
> 降级为其他 agent 的兜底路径。四条结论:
>
> 1. **插件的 `hooks/hooks.json` 不支持 `type: "http"`** —— 同一个插件里 `type: "command"`
>    正常触发,`type: "http"` 一次都不触发。合理:一个能把 `tool_input` POST 到任意 URL 的
>    可分享插件是数据外泄载体。
> 2. **skills 目录插件会自动加载**:`~/.claude/skills/<name>/` 放一个
>    `.claude-plugin/plugin.json` + `hooks/hooks.json`,下次会话直接生效,**不需要 install
>    步骤,不需要改任何 settings.json**。
> 3. **同一事件上多个阻塞式 hook 会全部触发,最后注册的那个说了算**。实测:deny 在前
>    allow 在后 → 放行;调换顺序 → 拒绝。
> 4. 项目级 `<cwd>/.claude/settings.json` 的 hooks 正常生效,是**测试**本功能的正确姿势 ——
>    全程不必碰全局配置。

**新方案**:octoterm 在 `~/.claude/skills/octoterm/` 下写**自己独占的一棵目录树**,
hook 用 `type: "command"` 指向 **octoterm 自己的二进制**(新增一个 `hook` 子命令)。

这一改消掉了原方案的大部分副作用:

| 原副作用 | 新方案下 |
| --- | --- |
| 改用户/别家共用的文件 | **消失** —— 目录整个是我们的 |
| 与 Claude Code 自己的写入竞态(它会往 `permissions` 加规则) | **消失** —— 不再 read-modify-write 别人的文件 |
| 重新序列化打乱用户文件格式 | **消失** |
| 备份文件落在用户目录里 | **消失** —— 不覆盖任何非我方文件,不需要备份 |
| 改端口后 URL 失效(`StalePort`) | **消失** —— 目录是我们的,每次 server 启动重写一遍即可,自愈 |
| 非托管会话打来 401、刷爆日志 | **消失** —— 我们的二进制发现环境里没有 `OCTOTERM_SESSION_ID` 就**直接退出**,零网络 |
| 端口被别人占用时把 `tool_input` 发过去 | **减轻** —— 由我们的二进制发起连接,可以先验明对端身份再发 payload |
| 全机所有 Claude 会话都会触发 hook | **仍在**,但代价降为一次极短的进程启动 |
| 与别家阻塞式 hook 抢同一事件 | **仍在**,见下面的 Step 5 |

代价:每个 hook 事件多一次进程启动(遥测类 `async: true`,不阻塞 Claude);二进制路径要写进
`hooks.json`(但目录是我们的,每次启动重写即可自愈)。

**Files:**
- Create: `crates/server/src/agent/apply.rs`(写/删我们独占的目录树)
- Create: `crates/server/src/agent/hook_cli.rs`(`hook` 子命令)
- Create: `crates/server/tests/agent_install.rs`
- Modify: `crates/server/src/agent/claude_code.rs`(`plan_install` 改为产出文件内容)
- Modify: `crates/server/src/agent/edit.rs`(`EditOp` 增加 `WriteOwnedFile` / `RemoveOwnedTree`)
- Modify: `crates/server/src/agent/detect.rs`(见 Step 4 的自证陷阱)
- Modify: `crates/server/src/main.rs`(子命令)、`config.rs`(`[agents]` 配置节)、`agent/routes.rs`

**Interfaces:**
- Produces:
  - `POST /api/agents/{id}/install`、`POST /api/agents/{id}/uninstall`
  - `GET /api/agents/{id}/plan` —— 预演,返回将要创建/删除的文件清单
  - `octoterm-server hook <agent> <event>` —— hook 子命令

- [ ] **Step 1: 写失败的测试**

```rust
#[test] fn install_disabled_writes_nothing() {}          // 开关关着 → 403,一个文件都不建
#[test] fn install_creates_only_our_own_tree() {}        // 只在 skills/octoterm/ 下建东西
#[test] fn install_is_idempotent_on_disk() {}            // 装两次,目录内容逐字节相同
#[test] fn uninstall_removes_our_tree_only() {}          // 兄弟目录(别的 skill)一个不动
#[test] fn install_refreshes_stale_binary_path() {}      // 二进制换了位置 → 重装后指向新路径
#[test] fn install_refuses_to_delete_foreign_tree() {}   // 目录存在但没有我们的 manifest → 拒绝,不覆盖
```

最后一条是底线:`~/.claude/skills/octoterm/` 万一是用户自己的东西,我们**宁可报错也不能删**。
判据是里面有没有我们写的 `.claude-plugin/plugin.json` 且 `name == "octoterm"`。

- [ ] **Step 2: 产出的目录树**

```
~/.claude/skills/octoterm/
├── .claude-plugin/plugin.json     { "name": "octoterm", "description": ..., "version": <本机 server 版本> }
└── hooks/hooks.json               全部 type:command,指向 octoterm 二进制
```

`hooks.json` 里一条长这样:

```json
{ "hooks": { "PermissionRequest": [ { "matcher": "", "hooks": [
  { "type": "command",
    "command": "\"/Applications/octoterm.app/Contents/MacOS/octoterm-server\" hook claude-code permission-request",
    "timeout": 600 } ] } ] } }
```

遥测类同形,但 `"timeout": 5` + `"async": true`。

二进制路径取 `std::env::current_exe()`。**每次 server 启动时无条件重写这棵树**(内容一致就
不落盘),于是换路径、换端口、升级版本全部自愈 —— 这是「目录归我们所有」换来的最大好处。

- [ ] **Step 3: `hook` 子命令**

```
octoterm-server hook <agent> <event>
```

行为,按顺序:

1. 读 `OCTOTERM_SESSION_ID` / `OCTOTERM_HOOK_TOKEN`。**任一缺失就立刻 exit 0,不打印、
   不联网** —— 这一条就是「只管托管会话」这条边界的执行点,也是非托管会话零噪声的原因。
2. 从 stdin 读 JSON(有上限,超限即放弃)。
3. POST 到 `http://127.0.0.1:<port>/hook/<agent>/<event>`,带 `Authorization` 头。
   端口从环境变量拿(`OCTOTERM_HOOK_PORT`,spawn 时一并注入),不去猜、不扫端口。
4. 遥测类:短超时,失败静默 exit 0 —— **绝不能因为宿主不在就影响 Claude**。
5. 决策类:阻塞等响应,把响应体原样打印到 stdout;超时/失败则**不打印任何东西**
   (= 无决定),让 Claude 回落到它自己的审批弹窗。

> ⚠️ 待实测:command hook 由 Claude Code spawn,应当继承 pty 的环境变量。原理上必然,
> 但第一步就要验证 `OCTOTERM_SESSION_ID` 确实读得到,否则整条链路不成立。

- [ ] **Step 4: 堵掉自证陷阱**

我们会在 `~/.claude/skills/` 下建目录,而 Task 1 的检测里「`~/.claude` 目录下有别的东西」
是「用户装过 Claude Code」的证据之一。装完 hook 之后,这条证据就变成了**我们自己造的**。

这正是参考实现栽过的坑(它自己创建 `~/.claude/`,最后不得不把 claude-code 整个从默认检测
里排除)。修法:`skills/` 只有在**除我们之外**还有别的条目时才算证据。加回归测试。

- [ ] **Step 5: 与别家阻塞式 hook 共存的策略**

实测结论 3:多个阻塞 hook 全部触发,**最后注册的赢**。而插件与 settings.json 的相对顺序
**尚未实测**。这意味着装上我们的 hook 可能会**悄悄推翻**别家(例如 clawd-on-desk)的 deny。

不赌顺序。策略:

- `GET /api/agents` 已经能报出 `conflicts`(Task 1 已实现,本机实测有效);
- **检测到别家阻塞式 hook 时,默认不装我们的决策类 hook**,只装遥测类,并在 UI 上说明
  原因与后果;用户显式要求才装。
- 遥测类无论如何都安全,不参与决策。

- [ ] **Step 6: 配置节与门控**

```toml
[agents]
install_enabled = false     # 默认关。写的虽然是自己的目录,但会改变全机 Claude 的行为
session_stale_secs = 600
working_stale_secs = 300
```

`Config` 加 `#[serde(default)] pub agents: AgentsConfig`,全部字段带默认值 —— 老配置文件
不写这一节也要能读。

- [ ] **Step 7: 改注释,消除「代码说 A、行为是 B」**

`launcher/mod.rs` 顶部「octoterm 从不写别人的配置文件」这句现在**基本还是真的** ——
新方案只写自己独占的目录,不改任何别人会写的文件。措辞按这个事实收紧,并指向 spec。
`config.rs` 的「server 自己永不写文件」同理。

`docs/protocol.md` 的 T10 修订照旧:`/api/` 下新增变更子集,分配新规则 ID。

- [ ] **Step 8: 验证**

测试全绿。手工:开关关着时 403 且零文件改动;打开后装一次、再装一次,目录逐字节相同;
卸载后 `~/.claude/skills/` 恢复原状;**全程 `~/.claude/settings.json` 的 mtime 不变**
(这一条要写成断言,它是本 task 的核心承诺)。

### Task 4: 协议扩展 —— `AgentEvent`

**Files:**
- Modify: `crates/protocol/src/messages.rs`、`crates/protocol/fixtures/server-msgs.json`
- Modify: `docs/protocol.md`(新增 agent 一节 + §10 限额 + §12.1 复用表补一行)
- Modify: `clients/web/src/protocol.ts`(如需要)

- [ ] **Step 1: 加消息类型**

```rust
/// agent 集成的状态广播。**只描述状态,不含任何窗口/标签/面板语义**(R13):
/// 客户端拿它渲染成什么样是客户端的事。
AgentEvent {
    agent_id: String,
    agent_session_id: String,
    /// 关联到的托管会话;agent 跑在 octoterm 之外时为 None(P1 里不会出现,
    /// 因为鉴权已经把外部会话挡掉了,但字段先留着,免得以后要 bump)
    session: Option<u64>,
    state: AgentState,
    /// 有值表示正在等人回答,值是 /api/agents/answer 的自然键(R5)
    pending: Option<String>,
    title: Option<String>,
},
```

`AgentState` 是 `idle | thinking | working | waiting | done | error`,kebab-case。

- [ ] **Step 2: 更新 fixtures 并让 roundtrip 测试通过**

`messages.rs` 里现成的 `fixtures_roundtrip` 测试会自动覆盖新类型 —— 前提是把样例加进 `server-msgs.json`。

- [ ] **Step 3: 文档**

`docs/protocol.md` 新增一节,内容至少包含:消息形状、何时广播、限额(≤ 4 KiB)、以及 §12.2 的 R1–R13 应答(spec 里已经写好,搬过来并落成正式规则 ID)。§12.1 的「复用优先」表补一行,免得下一个人重复问「为什么不用 session-event」。

- [ ] **Step 4: 验证**

`cargo test -p octoterm-protocol` 全绿。**特别确认 `PROTO_VERSION` 没被改动** —— 这是本 task 最容易手滑的地方。

---

### Task 5: 会话表、hook 摄入面与 env 注入

**Files:**
- Create: `crates/server/src/agent/store.rs`
- Create: `crates/server/tests/agent_hook.rs`
- Modify: `crates/server/src/session/pty.rs`(注入两个环境变量)、`agent/routes.rs`、`app.rs`

**Interfaces:**
- Produces:
  - `POST /hook/{agent}/{event}`
  - `store::{AgentSessionStore, AgentEvent, AgentState}`

- [ ] **Step 1: 写失败的测试**

```rust
#[test] fn 没有_authorization_头一律_401() {}
#[test] fn token 不对一律 401() {}
#[test] fn 未知 agent 返回 404 而不是 500() {}
#[test] fn 遥测事件推进状态并广播() {}
#[test] fn notification_permission_prompt_置为_waiting() {}
#[test] fn 未知事件名被忽略而不是报错() {}   // agent 升级会带来新事件,不能因此 500
```

- [ ] **Step 2: env 注入**

`pty.rs` 的 `spawn` 里,紧挨着现有的 `TERM` / `COLORTERM`:

```rust
cmd.env("OCTOTERM_SESSION_ID", id.to_string());
cmd.env("OCTOTERM_HOOK_TOKEN", hook_token);
```

`hook_token` 由 `AppState` 在**进程启动时随机生成一次**并传进来,不落盘、不轮换(见 spec:环境变量就是能力本身)。`Session::spawn` 的签名要多带一个参数,`SessionManager` 一路透传。

- [ ] **Step 3: 摄入路由**

`POST /hook/{agent}/{event}`:

1. 取 `Authorization: Bearer`,与 `hook_token` **定长比较**;不匹配 → 401 空体;
2. `X-Octoterm-Session` 解析成 `u64`,查得到托管会话才继续;
3. `find(agent)` → 404;
4. `adapter.parse(event, &body)` → 更新会话表 → 广播 `AgentEvent`;
5. 遥测类立即 200 空体(**不能拖**,Claude 那头 timeout 只有 5 秒)。

**只绑 127.0.0.1**:路由层判定 `ConnectInfo` 的对端地址,非回环直接 403。

- [ ] **Step 4: 验证**

集成测试全绿。手工:在 octoterm 里开一个会话跑 `claude`,`GET /api/agents` 能看到它,状态随敲字/跑工具变化。

---

### Task 6: 阻塞决策与回答路由

**Files:**
- Create: `crates/server/tests/agent_pending.rs`
- Modify: `agent/store.rs`(`PendingRequest`)、`agent/routes.rs`

**Interfaces:**
- Produces:
  - `POST /api/agents/answer` body `{ pending_id, decision: "allow"|"deny", message? }`

- [ ] **Step 1: 写失败的测试 —— 三条终结路径都要覆盖**

```rust
#[tokio::test] async fn 用户回答后_hook_拿到决策() {}
#[tokio::test] async fn 重复回答同一个_pending_返回_409_且不改变已定决策() {}   // R6
#[tokio::test] async fn agent_侧断连时挂起项被清除() {}                        // 最容易漏
#[tokio::test] async fn 没有客户端连着时超时回落为无决定() {}
```

**第三条是本 task 的核心**:axum 的 handler future 在连接关闭时会被 drop。挂起项必须靠 `Drop` guard 摘除,而不是靠超时兜底 —— 否则一个崩掉的 agent 会把条目留到 600 秒。

- [ ] **Step 2: 实现**

```rust
struct PendingGuard { store: Arc<AgentSessionStore>, id: PendingId }
impl Drop for PendingGuard {
    fn drop(&mut self) { self.store.remove_pending(&self.id); }
}
```

handler 骨架:

```rust
let (tx, rx) = tokio::sync::oneshot::channel();
let id = store.insert_pending(meta, tx);
let _guard = PendingGuard { store: store.clone(), id: id.clone() };
match rx.await {
    Ok(d)  => (StatusCode::OK, Json(adapter.render(&d))).into_response(),
    Err(_) => StatusCode::NO_CONTENT.into_response(),   // 无决定
}
```

`render` 按**实测生效**的形态产出(spec「已实测的线上契约」):

```json
{ "hookSpecificOutput": { "hookEventName": "PermissionRequest",
    "decision": { "behavior": "allow", "message": "..." } } }
```

`decision` 是**对象**不是字符串 —— 字符串形态实测不生效。

- [ ] **Step 3: 降级**

按 spec 的降级矩阵实现。**不为 headless 写特殊分支**(实测 `-p` 下该事件根本不触发)。原则:宁可无决定,也不代替用户 allow/deny。

- [ ] **Step 4: 验证**

四条测试全绿。手工端到端:托管会话里让 claude 跑一条需要授权的命令 → 浏览器出现「等你回答」→ 点允许 → 终端里命令执行,TUI 打出 `Allowed by PermissionRequest hook`。

---

### Task 7: stale 清理

**Files:**
- Modify: `agent/store.rs`
- Create: 清理决策的单测(纯函数,放 `store.rs` 的 `#[cfg(test)]` 即可)

- [ ] **Step 1: 写失败的测试**

```rust
#[test] fn 关联会话已死立即清除_优先于一切超时判断() {}
#[test] fn idle 基准取 updated_at 与 acked_at 的较大者() {}
#[test] fn 超时转 idle 时不刷新时间戳() {}   // 否则永远删不掉
```

第三条是 clawd 用 187 行纯函数换来的教训,直接采纳。

- [ ] **Step 2: 实现**

决策函数写成**纯函数**:`fn decide(now, session, cfg) -> Option<Sweep>`,副作用留在调用方。定时器 10 秒一跳。

- [ ] **Step 3: 验证**

单测全绿,且不引入任何时间相关的 flaky(测试里注入 `now`,不读系统时钟)。

---

### Task 8: web 客户端面板

**Files:**
- Create: `clients/web/src/agents.ts`
- Modify: `clients/web/src/main.ts`、`style.css`、`i18n.ts`

- [ ] **Step 1: 列表与状态**

会话列表上给跑着 agent 的会话加一个状态标记,`waiting` 用醒目样式。数据来自 `AgentEvent` 广播,页面加载时先 `GET /api/agents/sessions` 拉全量(重连后同理 —— **不做增量对账**,R6)。

- [ ] **Step 2: 回答面板**

两种回答并存:

- **结构化**:有 `pending` 时显示允许/拒绝按钮 → `POST /api/agents/answer`;
- **自由文本**:任何时候都可以往那个会话的 pty 写字节 —— 这条不需要 pending,也不需要 agent 装 hook,直接复用现有数据通道。

第二条是 octoterm 独有的能力,UI 上不要把它藏在 pending 后面。

- [ ] **Step 3: 设置页**

agent 列表 + 每行的装/卸按钮。装之前先调 `GET /api/agents/{id}/plan` 把将要做的改动显示给用户。`install_enabled` 关着时按钮禁用并说明原因。

- [ ] **Step 4: 验证**

`npm run typecheck` 与 `npm run test` 全绿;手工走一遍装 → 开会话 → 触发授权 → 手机上回答的完整链路。

---

## 完成标准

- [ ] `cargo test --workspace` 与 `cargo clippy --workspace -- -D warnings` 全绿
- [ ] `PROTO_VERSION` **未改动**
- [ ] 装一次 / 装两次 / 卸载,`~/.claude/settings.json` 分别满足:生效 / 逐字节幂等 / 逐字节还原
- [ ] server 停掉时,Claude Code 仍然正常可用(实测已证明是 non-blocking,回归时复验一次)
- [ ] octoterm 之外启动的 claude,其 hook 请求被 401 拒收
- [ ] `docs/protocol.md` 与 README 的路线图第 2 条同步更新
