//! Claude Code adapter。
//!
//! 装的全部是 `type: "http"` hook —— octoterm-server 自己就是那个 HTTP 端点,
//! 不需要随包携带任何脚本或 node 运行时。这和「单静态二进制」的定位是一回事,
//! 也是相对参考实现(必须打包 node + 一堆 hook js)的结构性优势。
//!
//! 鉴权与会话关联走同一个机制:hook 的 `headers` 支持 `$VAR` 插值,插值发生在
//! hook 触发那一刻、取自 Claude 进程的环境,而那份环境是 octoterm spawn 这个会话
//! 时给的。于是**环境变量就是能力本身** —— 在 octoterm 之外启动的 Claude 拿不到
//! 这两个变量,hook 照样触发,但没有 `Authorization` 头,一律 401 拒收。

use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;

use super::detect::{self, DetectEnv};
use super::edit::{is_ours, slug_of_event, ConfigEdit, EditOp, InstallCtx};
use super::store::{Decision, Update};
use super::{AgentAdapter, Confidence, Detected, Integration};
use octoterm_protocol::AgentState;

pub struct ClaudeCode;

/// 遥测类:只用来推进会话状态,不参与决策。`async: true` + 短超时,绝不拖住 Claude。
const TELEMETRY: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
    "Notification",
];

/// 决策类:Claude 会阻塞在 socket 上等我们写响应,最长 `timeout` 秒。
const BLOCKING: &[&str] = &["PermissionRequest"];

const TELEMETRY_TIMEOUT_SECS: u64 = 5;
/// 给人类反应时间。这个值同时是「一个挂起请求最长活多久」的上界。
const BLOCKING_TIMEOUT_SECS: u64 = 600;

fn settings_path(home: &std::path::Path) -> PathBuf {
    // 已知缺口:Claude Code 支持用 $CLAUDE_CONFIG_DIR 换掉 ~/.claude。P1 不处理,
    // 因为它会让「装到哪、卸哪个」多出一条要跟着环境变量走的路径。
    home.join(".claude").join("settings.json")
}

fn hook_spec(port: u16, event: &str, blocking: bool) -> Value {
    let url = format!("http://127.0.0.1:{port}/hook/claude-code/{}", slug_of_event(event));
    let mut spec = json!({
        "type": "http",
        "url": url,
        "headers": {
            "Authorization": "Bearer $OCTOTERM_HOOK_TOKEN",
            "X-Octoterm-Session": "$OCTOTERM_SESSION_ID",
        },
        "allowedEnvVars": ["OCTOTERM_HOOK_TOKEN", "OCTOTERM_SESSION_ID"],
    });
    let obj = spec.as_object_mut().expect("字面量就是对象");
    if blocking {
        obj.insert("timeout".into(), json!(BLOCKING_TIMEOUT_SECS));
    } else {
        obj.insert("timeout".into(), json!(TELEMETRY_TIMEOUT_SECS));
        // 遥测的输出会被忽略,不该占住 Claude 的主循环
        obj.insert("async".into(), json!(true));
    }
    spec
}

fn managed_events() -> impl Iterator<Item = (&'static str, bool)> {
    TELEMETRY.iter().map(|e| (*e, false)).chain(BLOCKING.iter().map(|e| (*e, true)))
}

/// 装的时候按开关过滤;**卸载永远覆盖全部事件** —— 否则关掉开关再卸载会留下残留。
fn events_to_install(ctx: &InstallCtx) -> impl Iterator<Item = (&'static str, bool)> + '_ {
    managed_events().filter(move |(_, blocking)| ctx.include_blocking || !blocking)
}

fn read_settings(path: &std::path::Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

impl AgentAdapter for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn name(&self) -> &'static str {
        "Claude Code"
    }

    fn detect(&self, env: &DetectEnv) -> Detected {
        let dir = env.home.join(".claude");
        let settings = settings_path(&env.home);

        // 1. 配置文件里有非我方内容 —— 最硬的证据
        if let Some(v) = read_settings(&settings)
            && has_foreign_content(&v)
        {
            return Detected {
                installed: true,
                confidence: Confidence::High,
                reason: "config-file",
                detail: format!("{} 里有用户自己的配置", settings.display()),
                config_path: Some(settings),
            };
        }

        // 2. CLI 在 PATH 上。注意**不执行** `claude --version`:扫描是只读操作,
        //    不 spawn 进程。
        if let Some(exe) = detect::on_path(env, "claude") {
            return Detected {
                installed: true,
                confidence: Confidence::High,
                reason: "cli-path",
                detail: format!("PATH 上找到 {}", exe.display()),
                config_path: Some(settings),
            };
        }

        // 3. 目录里有 settings.json 以外的东西 —— 用户跑过才会留下
        if dir.is_dir() && detect::dir_has_entries_besides(&dir, &["settings.json"]) {
            return Detected {
                installed: true,
                confidence: Confidence::Medium,
                reason: "parent-dir",
                detail: format!("{} 下有使用痕迹", dir.display()),
                config_path: Some(settings),
            };
        }

        Detected {
            installed: false,
            confidence: Confidence::Low,
            reason: "not-found",
            detail: "没有找到 Claude Code 的安装痕迹".into(),
            config_path: None,
        }
    }

    fn plan_install(&self, ctx: &InstallCtx) -> Result<Vec<ConfigEdit>> {
        let path = settings_path(&ctx.home);
        Ok(events_to_install(ctx)
            .map(|(event, blocking)| ConfigEdit {
                path: path.clone(),
                op: EditOp::EnsureHook {
                    event: event.to_string(),
                    // Claude Code 的 group 带 matcher(空串 = 匹配全部)
                    group: json!({ "matcher": "", "hooks": [hook_spec(ctx.port, event, blocking)] }),
                },
            })
            .collect())
    }

    fn plan_uninstall(&self, ctx: &InstallCtx) -> Result<Vec<ConfigEdit>> {
        let path = settings_path(&ctx.home);
        Ok(managed_events()
            .map(|(event, _)| ConfigEdit {
                path: path.clone(),
                op: EditOp::RemoveOurs { event: event.to_string() },
            })
            .collect())
    }

    fn parse(&self, event: &str, body: &Value) -> Option<Update> {
        let state = state_of(event)?;
        let detail = match event {
            "PreToolUse" | "PostToolUse" => str_field(body, "tool_name"),
            // notification_type 就是 matcher 的取值:permission_prompt / idle_prompt /
            // agent_needs_input / elicitation_dialog ...
            "Notification" => str_field(body, "notification_type").or(Some("需要你的输入".into())),
            _ => None,
        };
        Some(Update {
            state: Some(state),
            detail,
            cwd: str_field(body, "cwd"),
            title: str_field(body, "session_title"),
        })
    }

    fn is_blocking(&self, event: &str) -> bool {
        BLOCKING.contains(&event)
    }

    /// **`decision` 是对象,不是字符串。**
    ///
    /// 调研阶段拿到的二手资料说它是 `"allow"|"deny"|"escalate"|"ask"` 字符串,实测
    /// 不生效 —— 字符串形态下审批弹窗照常出现,换成对象形态后 TUI 立刻打出
    /// `Allowed by PermissionRequest hook`。以实测为准。
    ///
    /// 「无决定」返回空对象:官方文档写明「2xx + 空 body = 成功且无输出」,Claude 会
    /// 回落到它自己的审批弹窗。这正是我们要的降级 —— 把选择权交还给终端前的人。
    fn render(&self, decision: &Decision) -> Value {
        let d = match decision {
            Decision::NoDecision => return json!({}),
            Decision::Allow { message, updated_input } => {
                let mut o = json!({ "behavior": "allow" });
                if let Some(m) = message {
                    o["message"] = json!(m);
                }
                // 选择题的答案就藏在这里:放行的同时把改写后的入参交回去
                if let Some(u) = updated_input {
                    o["updatedInput"] = u.clone();
                }
                o
            }
            Decision::Deny { message } => {
                let mut o = json!({ "behavior": "deny" });
                if let Some(m) = message {
                    o["message"] = json!(m);
                }
                o
            }
        };
        json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": d } })
    }

    fn integration(&self, ctx: &InstallCtx) -> (Integration, Vec<String>) {
        let Some(doc) = read_settings(&settings_path(&ctx.home)) else {
            return (Integration::NotInstalled, Vec::new());
        };
        let mut ours_here = 0usize;
        let mut ours_elsewhere = 0usize;
        let mut conflicts = Vec::new();

        for (event, blocking) in managed_events() {
            for hook in hooks_of(&doc, event) {
                if is_ours(hook, ctx.port) {
                    ours_here += 1;
                } else if is_our_shape(hook) {
                    ours_elsewhere += 1;
                } else if blocking && hook.get("type").and_then(Value::as_str) == Some("http") {
                    // 别人的阻塞式 hook 挂在同一个事件上。不动它,但要报出来。
                    let url = hook.get("url").and_then(Value::as_str).unwrap_or("<无 url>");
                    conflicts.push(format!("{event} 上已有另一个阻塞式 hook:{url}"));
                }
            }
        }

        let state = if ours_here > 0 {
            Integration::Installed
        } else if ours_elsewhere > 0 {
            Integration::StalePort
        } else {
            Integration::NotInstalled
        };
        (state, conflicts)
    }
}

/// 事件 → 状态。
///
/// `Stop` 映射成 `Idle` 而不是「等你」:一个回合结束不等于它要你做什么。真正的
/// 「在等人」只从 `Notification` 来(它的 matcher 里有 `permission_prompt` /
/// `idle_prompt` / `agent_needs_input`),以及 Task 6 接上的挂起请求。这样
/// `Waiting` 才是个有信息量的状态,而不是每轮都亮一次的噪声。
fn state_of(event: &str) -> Option<AgentState> {
    Some(match event {
        "SessionStart" => AgentState::Idle,
        "SessionEnd" => AgentState::Done,
        "UserPromptSubmit" => AgentState::Thinking,
        "PreToolUse" | "PostToolUse" => AgentState::Working,
        "Stop" => AgentState::Idle,
        "Notification" => AgentState::Waiting,
        _ => return None,
    })
}

fn str_field(body: &Value, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).map(str::to_string)
}

/// 我方形状但端口对不上 —— 用来识别「改过监听端口之后的残留」。
fn is_our_shape(hook: &Value) -> bool {
    hook.get("type").and_then(Value::as_str) == Some("http")
        && hook
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|u| u.starts_with("http://127.0.0.1:") && u.contains("/hook/claude-code/"))
}

fn hooks_of<'a>(doc: &'a Value, event: &str) -> impl Iterator<Item = &'a Value> {
    doc.get("hooks")
        .and_then(|h| h.get(event))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| g.get("hooks"))
        .filter_map(Value::as_array)
        .flatten()
}

/// 配置里有没有「不是我们写的」内容。只有我方 hook 的文件证明不了 Claude Code 存在。
fn has_foreign_content(doc: &Value) -> bool {
    let Some(obj) = doc.as_object() else { return false };
    if obj.keys().any(|k| k != "hooks") {
        return true;
    }
    let Some(hooks) = obj.get("hooks").and_then(Value::as_object) else { return false };
    hooks.values().any(|groups| {
        groups
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|g| g.get("hooks"))
            .filter_map(Value::as_array)
            .flatten()
            .any(|h| !is_our_shape(h))
    })
}
