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

### Task 3: 落盘执行与安装路由

照参考实现的形态:**写 `~/.claude/settings.json`,hook 用 `type: "http"` 直连**。
Claude 直接 POST 到 octoterm-server,没有中间脚本、没有进程启动 —— server 自己就是
那个端点,这和「单静态二进制」的定位是一回事。

> **插件目录方案已评估并否决**,理由是实测硬约束:**插件里 `type: "http"` 根本不触发**,
> 只能用 `type: "command"`,连带要求新增 `hook` 子命令 + 每事件 spawn 一次进程,还会让
> octoterm 出现在用户的 skills 列表里。换来的好处大部分能用工程手段压掉(见 spec
> 「已评估并否决的备选」)。这条实测结论留在文档里,免得以后再走一遍。

Task 2 已经写好并测过的编辑计划、所有权判定、幂等与还原,**就是这一 task 的地基**。
这里只加「怎么安全地落到磁盘上」。

**Files:**
- Create: `crates/server/src/agent/apply.rs`
- Create: `crates/server/tests/agent_install.rs`
- Modify: `crates/server/src/config.rs`(`[agents]` 配置节)、`agent/routes.rs`、`app.rs`
- Modify: `crates/server/src/launcher/mod.rs`(顶部注释措辞)、`docs/protocol.md`(T10 修订)

**Interfaces:**
- Produces:
  - `GET  /api/agents/{id}/plan` —— 预演,返回将要产生的编辑与 diff 摘要
  - `POST /api/agents/{id}/install`、`POST /api/agents/{id}/uninstall`

- [x] **Step 1: 写失败的测试**

`crates/server/tests/agent_install.rs`,用 `tempfile` 造假 home:

```rust
#[test] fn disabled_switch_writes_nothing() {}        // 开关关着 → 403,文件字节不变
#[test] fn backup_lands_outside_target_dir() {}       // 备份不在 ~/.claude 里
#[test] fn backup_matches_original_byte_for_byte() {}
#[test] fn backup_keeps_only_five() {}                // 装 7 次,只剩 5 份
#[test] fn write_is_atomic() {}                       // tmp + rename,不存在半截文件
#[test] fn refuses_when_target_is_not_valid_json() {} // 宁可报错也不覆盖
#[test] fn install_then_uninstall_is_byte_identical() {} // 端到端落盘版的还原保证
#[test] fn missing_target_file_is_created() {}        // 用户还没有 settings.json 的情形
```

- [x] **Step 2: 配置节**

```toml
[agents]
install_enabled = false     # 默认关。headless / 共享部署可永久关闭
session_stale_secs = 600
working_stale_secs = 300
```

`Config` 加 `#[serde(default)] pub agents: AgentsConfig`,全部字段带默认值 —— 老配置
文件不写这一节也要能读。

- [x] **Step 3: 落盘**

顺序不能变:

```
门控 → 读原文 → 解析成 JSON(失败即 bail)→ 应用计划 → 备份原文 → tmp 写入 → rename
```

备份在**解析成功之后、写入之前**:为一次注定失败的编辑留下备份只是垃圾。

两条与参考实现不同的地方,都是有意的:

1. **备份落在 octoterm 自己的配置目录**,不是就地写 `.bak`。`~/.claude` 是别人的地方,
   我们不往里堆垃圾。
2. **只在 install / uninstall 时写,不做「每次 server 启动重写」**。Claude Code 自己也
   会写这个文件(「Yes, and always allow」会往 `permissions` 里加规则),读-改-写的竞态
   窗口必须只出现在用户显式动作那一刻。端口变了导致的失效交给 `StalePort` 自检
   (Task 1 已实现)去报,由用户点「修复」,而不是我们背着他反复改文件。

- [x] **Step 4: 三条路由**

`plan` 是只读的,返回将要做的编辑;`install` / `uninstall` 是变更。变更类要过门控,
且必须返回**做了什么**(改了哪个文件、备份在哪),不能只回一个 200。

- [x] **Step 5: 与别家阻塞式 hook 共存**

实测:同一事件上多个阻塞 hook **全部触发,最后注册的赢**(deny 在前 allow 在后 → 放行;
调换顺序 → 拒绝)。我们的计划把自己的组 append 到数组末尾,**因此会覆盖别家的决策** ——
本机同时装了 clawd-on-desk 时就会发生。

不赌顺序,也不偷偷占先:

- `GET /api/agents` 已能报出 `conflicts`(Task 1 实现,本机实测有效);
- **检测到别家阻塞式 hook 时,默认只装遥测类**,决策类要用户显式确认,并说清后果;
- 遥测类无论如何都安全,不参与决策。

- [x] **Step 6: 改注释与协议文档**

`launcher/mod.rs` 顶部「octoterm 从不写别人的配置文件」改成限定作用域的措辞:**发现只读,
集成需用户显式动作**,并指向 spec。`config.rs` 的「server 自己永不写文件」同理。

`docs/protocol.md` 的 T10 修订:`/api/` 下**只读子集**保持 GET/无状态/幂等,新增
`/api/agents/*` 变更子集,说明它为什么不属于会话/通道语义。分配新规则 ID(不复用、不重编号)。

- [x] **Step 7: 验证**

测试全绿。手工三连:

1. 开关关着时 `install` 返回 403,`~/.claude/settings.json` 的 mtime 不变;
2. 打开后装一次、再装一次 —— 文件**逐字节相同**(幂等);
3. 卸载 —— 与初始文件**逐字节相同**(还原)。

第 2、3 条用真实的 `~/.claude/settings.json` 副本跑,不是合成数据。


---

## 实施记录:Task 3(2026-08-19,已完成)

`agent_install` 9 个用例 + 端到端验证全过,`cargo test --workspace` 222 通过 0 失败,
`cargo clippy --workspace -- -D warnings` 干净。

**端到端用的正是「测试目录」这条路子**:把 server 的 `HOME` 指向测试目录,安装器产出的
`<home>/.claude/settings.json` 落点恰好就是**项目级配置**的位置 —— 用户级和项目级共用
同一套 `hooks` schema,所以真实 Claude 读到的就是我们真正生成的那份文件。全程
`~/.claude/settings.json` 的 mtime 停在 8 月 17 日没动过。

端到端结果(真实 Claude,haiku):

```
/hook/claude-code/user-prompt-submit  auth='Bearer secret-abc123'  session='42'
/hook/claude-code/pre-tool-use        auth='Bearer secret-abc123'  session='42'
/hook/claude-code/post-tool-use       auth='Bearer secret-abc123'  session='42'
/hook/claude-code/stop                auth='Bearer secret-abc123'  session='42'
```

- **解决了 Task 2 留的待验证项**:`async: true` 与 header 的 `$VAR` 插值**共存正常**,
  遥测类不必退回同步形态。
- 鉴权头与会话头都正确插值,事件名 → 路径 slug 的转写也对得上。
- `-p` 模式下没看到 `SessionStart` / `SessionEnd` / `Notification`。不阻塞,但 Task 5 接
  摄入面时要留意:**不能假设 `SessionStart` 一定先到**,会话表要能被任意事件惰性创建。

**真实字节校验**(种子是那份别家装满 hook 的脱敏 fixture):

| 检查 | 结果 |
| --- | --- |
| 冲突时自动降级 | `include_blocking=false`,只装 7 个遥测事件,`PermissionRequest` 被排除 ✓ |
| 装第一次 | `changed=true`,文件确实变化 ✓ |
| 装第二次 | `changed=false`,逐字节相同 ✓ |
| 卸载 | 逐字节还原 ✓ |
| 备份落点 | `<config>/octoterm/agent-backups/`,**不在 `.claude` 里** ✓ |
| 开关关闭 | `install` 返回 403,文件 mtime 未变 ✓ |

**一个测出来的边界**:上面的「逐字节还原」成立的前提是原文格式与我们的 render 一致
(2 空格缩进 + 末尾换行,也就是 Claude Code 自己写出来的形状)。实测把文件改成 4 空格
缩进后再装再卸,结果是**语义还原、格式被规整成 2 空格**。这就是 spec 里列的副作用⑥,
现在是实测值而不是推断。可接受,但 UI 上装之前应当提一句。

**与计划的偏差**:

1. **`InstallCtx` 多了 `include_blocking`**。Step 5 的冲突策略需要它,而放进 ctx 比改 trait
   签名干净。卸载**永远**覆盖全部事件 —— 否则关掉开关再卸载会留下残留。
2. **`Supervisor::new` 多了一个参数**(`AgentsConfig`),desktop 的 10 处测试构造点同步更新。
3. `describe()` 把 `EditOp` 投影成 `{path, action, event, spec}` 再返回,不直接序列化内部
   枚举 —— 客户端不该依赖服务端的内部结构(R13)。

**抓到并修掉的真 bug**:`serde_json::Map::remove` 在 `preserve_order` 下是 **swap_remove**
语义,会把最后一个键换到被删的位置,顺序当场就乱。第二次安装因此产生了不同的字节。
必须用 `shift_remove`。

值得记的是**为什么之前没抓到**:JSON 层的 `install_is_idempotent` 是绿的 —— `IndexMap`
的相等比较**与顺序无关**,`Value == Value` 看不出键序变化。只有字节级的
`second_install_is_byte_identical_and_skips_write` 能抓。凡是承诺「保持用户文件原样」的
地方,断言必须落在字节上,不能落在语义上。

### Task 4: 协议扩展 —— `AgentEvent`

**Files:**
- Modify: `crates/protocol/src/messages.rs`、`crates/protocol/fixtures/server-msgs.json`
- Modify: `docs/protocol.md`(新增 agent 一节 + §10 限额 + §12.1 复用表补一行)
- Modify: `clients/web/src/protocol.ts`(如需要)

- [x] **Step 1: 加消息类型**

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

- [x] **Step 2: 更新 fixtures 并让 roundtrip 测试通过**

`messages.rs` 里现成的 `fixtures_roundtrip` 测试会自动覆盖新类型 —— 前提是把样例加进 `server-msgs.json`。

- [x] **Step 3: 文档**

`docs/protocol.md` 新增一节,内容至少包含:消息形状、何时广播、限额(≤ 4 KiB)、以及 §12.2 的 R1–R13 应答(spec 里已经写好,搬过来并落成正式规则 ID)。§12.1 的「复用优先」表补一行,免得下一个人重复问「为什么不用 session-event」。

- [x] **Step 4: 验证**

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

- [x] **Step 1: 写失败的测试**

```rust
#[test] fn 没有_authorization_头一律_401() {}
#[test] fn token 不对一律 401() {}
#[test] fn 未知 agent 返回 404 而不是 500() {}
#[test] fn 遥测事件推进状态并广播() {}
#[test] fn notification_permission_prompt_置为_waiting() {}
#[test] fn 未知事件名被忽略而不是报错() {}   // agent 升级会带来新事件,不能因此 500
```

- [x] **Step 2: env 注入**

`pty.rs` 的 `spawn` 里,紧挨着现有的 `TERM` / `COLORTERM`:

```rust
cmd.env("OCTOTERM_SESSION_ID", id.to_string());
cmd.env("OCTOTERM_HOOK_TOKEN", hook_token);
```

`hook_token` 由 `AppState` 在**进程启动时随机生成一次**并传进来,不落盘、不轮换(见 spec:环境变量就是能力本身)。`Session::spawn` 的签名要多带一个参数,`SessionManager` 一路透传。

- [x] **Step 3: 摄入路由**

`POST /hook/{agent}/{event}`:

1. 取 `Authorization: Bearer`,与 `hook_token` **定长比较**;不匹配 → 401 空体;
2. `X-Octoterm-Session` 解析成 `u64`,查得到托管会话才继续;
3. `find(agent)` → 404;
4. `adapter.parse(event, &body)` → 更新会话表 → 广播 `AgentEvent`;
5. 遥测类立即 200 空体(**不能拖**,Claude 那头 timeout 只有 5 秒)。

**只绑 127.0.0.1**:路由层判定 `ConnectInfo` 的对端地址,非回环直接 403。

- [x] **Step 4: 验证**

集成测试全绿。手工:在 octoterm 里开一个会话跑 `claude`,`GET /api/agents` 能看到它,状态随敲字/跑工具变化。

---


---

## 实施记录:Task 4 / Task 5(2026-08-19,已完成)

`agent_hook` 10 个用例,`cargo test --workspace` 232 通过 0 失败,clippy 干净。
**`PROTO_VERSION` 仍是 1**。

**广播通路是白捡的**:`conn.rs` 里那个事件转发循环把 `manager.events()` 收到的
**任何** `ServerMsg` 转给所有连接。给 `SessionManager` 加一个 `publish()` 就够了,
`conn.rs` 一行没改。

**鉴权是两套凭据、两个信任域**,有专门的回归测试
(`client_token_does_not_work_on_hook_plane`):客户端那个 bearer token 打不动 hook 面,
反之亦然。hook 密钥是进程级 `OnceLock`,不落盘 —— 因为写进 `settings.json` 的是字面量
`$OCTOTERM_HOOK_TOKEN`,插值发生在 hook 触发那一刻、取自会话的环境。

**`/hook/*` 只认回环**:`serve()` 改用 `into_make_service_with_connect_info`,handler
拿到对端地址,非回环直接 403 —— 主监听是不是 `0.0.0.0` 都一样。那条路上跑的是
`tool_input`(命令原文、文件路径)。

**状态映射的一个取舍**:`Stop` 映射成 `Idle` 而不是「等你」。一个回合结束不等于它要你
做什么;真正的 `Waiting` 只从 `Notification` 来(matcher 里有 `permission_prompt` /
`idle_prompt` / `agent_needs_input`),以及 Task 6 的挂起请求。这样 `Waiting` 才有信息量,
而不是每轮都亮一次的噪声。

**兑现了 Task 3 留下的提示**:不假设 `SessionStart` 先到,任何事件都能惰性建出会话,
有专门用例 `any_event_can_create_the_session`。另外「只带 tool_name 的事件不该把 cwd
抹掉」也单独测了。

**未知事件返回 200 而不是 4xx**:agent 升级会带来新事件,不能因为多一个事件名就报错,
更不能把 Claude 卡住。

**补做了 Task 3 漏掉的一步**:Step 6 的两处注释当时勾了但没真改。现在改了 ——
`launcher/mod.rs` 的「只读是硬约束」限定到**发现**这个作用域,`config.rs` 的「server 永不
写文件」改成「自己的 config.toml 只读;唯一会写的是 agent 集成改的别人的配置」。
`docs/protocol.md` 的来源地图也补了 agent 模块两行。

**协议文档**:T10 拆成只读子集 + `T10a` 变更子集;T13 路由表补 5 行;§6.2 消息表补
`agent-event`;§10 补两条限额;§12.1 复用表补一行(说明为什么不是 `session-event`);
新增 §15「Agent integration [A]」8 条规则。其中 A8 明确划界:**server↔agent 那一侧
(装 hook、`/hook/*` 摄入)不属于本文档** —— 它不在客户端与 octoterm 之间的线上,
客户端既看不见也不依赖它。

### Task 6: 阻塞决策与回答路由

**Files:**
- Create: `crates/server/tests/agent_pending.rs`
- Modify: `agent/store.rs`(`PendingRequest`)、`agent/routes.rs`

**Interfaces:**
- Produces:
  - `POST /api/agents/answer` body `{ pending_id, decision: "allow"|"deny", message? }`

- [x] **Step 1: 写失败的测试 —— 三条终结路径都要覆盖**

```rust
#[tokio::test] async fn 用户回答后_hook_拿到决策() {}
#[tokio::test] async fn 重复回答同一个_pending_返回_409_且不改变已定决策() {}   // R6
#[tokio::test] async fn agent_侧断连时挂起项被清除() {}                        // 最容易漏
#[tokio::test] async fn 没有客户端连着时超时回落为无决定() {}
```

**第三条是本 task 的核心**:axum 的 handler future 在连接关闭时会被 drop。挂起项必须靠 `Drop` guard 摘除,而不是靠超时兜底 —— 否则一个崩掉的 agent 会把条目留到 600 秒。

- [x] **Step 2: 实现**

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

- [x] **Step 3: 降级**

按 spec 的降级矩阵实现。**不为 headless 写特殊分支**(实测 `-p` 下该事件根本不触发)。原则:宁可无决定,也不代替用户 allow/deny。

- [x] **Step 4: 验证**

四条测试全绿。手工端到端:托管会话里让 claude 跑一条需要授权的命令 → 浏览器出现「等你回答」→ 点允许 → 终端里命令执行,TUI 打出 `Allowed by PermissionRequest hook`。

---


---

## 实施记录:Task 6(2026-08-19,已完成)

`agent_pending` 7 个用例,`cargo test --workspace` 239 通过 0 失败,clippy 干净。

**端到端跑通了完整回路**(真实 Claude,交互式会话由 pty 驱动,"远程客户端"用 curl 扮演):

```
挂起请求: f309b1cc | 工具: Bash | 入参: {"command": "touch ./marker-loop", ...}
会话状态: waiting | pending: f309b1cc
客户端回答 allow → 200
Claude 那头: ⎿ Allowed by PermissionRequest hook    marker 文件已创建
```

也就是说:Claude 请求授权 → octoterm 把 HTTP 响应挂起 → 客户端看到挂起请求和确切命令
→ 回答 → Claude 继续执行。这是这个功能的全部意义所在。

**核心是那个 `Drop` guard**,`agent_disconnect_clears_the_pending_entry` 专门测它:agent 侧
断开时 axum 直接丢掉 handler 的 future,`await` 之后一行都不会跑。只有 `Drop` 能保证挂起项
被摘掉,否则它会一直挂到 590 秒,而客户端上显示一个永远等不到答复的「有事找你」。

**测试逼出来的一个设计**:`second_answer_is_conflict_not_found` 第一次跑是红的 —— 拿到 404
而不是 409。原因是用户答完之后 handler 立刻醒来、写响应、guard 摘掉条目,中间只有微秒级
窗口。修法不是去缩小那个窗口,而是**记住最近答过的 64 个 id**:客户端因为网络抖动重试一次,
应当拿到「你已经答过了」而不是「这个请求根本不存在」—— 这两件事对客户端不是一回事。

**降级严格遵守「宁可不作决定」**:超时、没人连着、答复端被丢弃,一律返回空对象 `{}`。
按官方文档「2xx + 空 body = 成功且无输出」,Claude 会回落到它自己的审批弹窗 —— 把选择权
交还给终端前的人,而不是替他 deny。`timeout_falls_back_to_no_decision` 断言的就是这个,
断言的是 `{}` 而不是 deny。

**`render()` 用的是实测形态**:`decision` 是**对象** `{behavior, message?}`,不是字符串。
调研阶段的二手资料说是字符串,实测不生效(弹窗照常出现)。

**新增**:`hook_token()` 支持用同名环境变量固定。端到端测试需要外部起的 Claude 拿到和
server 一致的值;对想固定的部署也有用。不设时仍是每进程随机。

**踩到的一个坑(与代码无关但值得记)**:第一次端到端全程静默,一个 hook 都没打进来 ——
新建的测试目录触发了 Claude 的工作区信任对话框,交互式会话卡在那里。先用 `claude -p`
跑一次建立信任,再驱动交互式会话就正常了。

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
