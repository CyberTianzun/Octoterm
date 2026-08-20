//! Codex adapter。
//!
//! 与 Claude Code 的三处关键差异,都是实测出来的:
//!
//! 1. **Codex 不支持 `type: "http"`**。它的 `HookHandlerConfig` 只有
//!    command / prompt / agent 三个变体(从 codex 二进制的类型名读出来)。所以这里的
//!    hook 指向 octoterm 自己的二进制(`hook <url>` 子命令,见 [`super::hook_cli`]),
//!    由它转发到本地 server。
//! 2. **配置在 `~/.codex/hooks.json`**,不在 config.toml 里,而且 group **不带
//!    `matcher`**(本机上被 Codex 接受并信任过的那份就是这个形状)。
//! 3. **装了不等于生效**:Codex 用 `trusted_hash` 逐条门控 ——
//!    `~/.codex/config.toml` 里长这样:
//!
//!    ```toml
//!    [hooks.state."/Users/x/.codex/hooks.json:pre_tool_use:0:0"]
//!    trusted_hash = "sha256:..."
//!    ```
//!
//!    用户必须在 Codex 的 TUI 里跑 `/hooks` review 之后,那些条目才会被写进去。
//!    **我们绝不替它写**:那是 Codex 有意设的闸,防的正是「第三方悄悄让 Codex 执行
//!    任意命令」。我们只负责把 hook 写好,并把这一步告诉用户。

use anyhow::Result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::detect::{self, DetectEnv};
use super::edit::{is_ours, slug_of_event, ConfigEdit, EditOp, InstallCtx};
use super::store::{Decision, Update};
use super::{AgentAdapter, Confidence, Detected, Integration};
use octoterm_protocol::AgentState;

pub struct Codex;

/// 只装本机上已被 Codex 接受过的那六个事件。
///
/// 二进制里能看到更多事件名(SessionEnd、SubagentStart、PreCompact…),但没实测过,
/// 而多装一个不被接受的事件可能让整份 hooks.json 被拒。宁可少,不冒险。
const TELEMETRY: &[&str] = &["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"];
const BLOCKING: &[&str] = &["PermissionRequest"];

const TELEMETRY_TIMEOUT_SECS: u64 = 30;
const BLOCKING_TIMEOUT_SECS: u64 = 600;

fn home_dir(home: &Path) -> PathBuf {
    home.join(".codex")
}

fn hooks_path(home: &Path) -> PathBuf {
    home_dir(home).join("hooks.json")
}

/// 命令串:`"<octoterm 二进制>" hook <url>`。
///
/// 二进制路径用 `current_exe()`。它会跟着安装位置走,所以「换了个地方放二进制」之后
/// 需要重装一次 —— 和端口变了要重装是同一类问题,由 `StalePort` 之外的自检去报
/// (目前只报端口,路径漂移是已知缺口)。
fn command_for(port: u16, event: &str) -> String {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "octoterm-server".into());
    let url = format!("http://127.0.0.1:{port}/hook/codex/{}", slug_of_event(event));
    format!("\"{exe}\" hook {url}")
}

fn hook_spec(port: u16, event: &str, blocking: bool) -> Value {
    json!({
        "type": "command",
        "command": command_for(port, event),
        "timeout": if blocking { BLOCKING_TIMEOUT_SECS } else { TELEMETRY_TIMEOUT_SECS },
    })
}

fn managed_events() -> impl Iterator<Item = (&'static str, bool)> {
    TELEMETRY.iter().map(|e| (*e, false)).chain(BLOCKING.iter().map(|e| (*e, true)))
}

fn events_to_install(ctx: &InstallCtx) -> impl Iterator<Item = (&'static str, bool)> + '_ {
    managed_events().filter(move |(_, blocking)| ctx.include_blocking || !blocking)
}

fn read_hooks(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn state_of(event: &str) -> Option<AgentState> {
    Some(match event {
        "SessionStart" => AgentState::Idle,
        "SessionEnd" => AgentState::Done,
        "UserPromptSubmit" => AgentState::Thinking,
        "PreToolUse" | "PostToolUse" => AgentState::Working,
        "Stop" => AgentState::Idle,
        _ => return None,
    })
}

impl AgentAdapter for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn name(&self) -> &'static str {
        "Codex"
    }

    fn detect(&self, env: &DetectEnv) -> Detected {
        let dir = home_dir(&env.home);
        let config = dir.join("config.toml");
        // `~/.codex/config.toml` 是 Codex 自己写的,我们从不碰它 —— 它存在就是硬证据。
        // (对比 Claude Code:那边 `~/.claude/` 是我们也会写的,所以不能这么判。)
        if config.is_file() {
            return Detected {
                installed: true,
                confidence: Confidence::High,
                reason: "config-file",
                detail: format!("{} 存在", config.display()),
                config_path: Some(hooks_path(&env.home)),
            };
        }
        if let Some(exe) = detect::on_path(env, "codex") {
            return Detected {
                installed: true,
                confidence: Confidence::High,
                reason: "cli-path",
                detail: format!("PATH 上找到 {}", exe.display()),
                config_path: Some(hooks_path(&env.home)),
            };
        }
        if dir.is_dir() && detect::dir_has_entries_besides(&dir, &["hooks.json"]) {
            return Detected {
                installed: true,
                confidence: Confidence::Medium,
                reason: "parent-dir",
                detail: format!("{} 下有使用痕迹", dir.display()),
                config_path: Some(hooks_path(&env.home)),
            };
        }
        Detected {
            installed: false,
            confidence: Confidence::Low,
            reason: "not-found",
            detail: "没有找到 Codex 的安装痕迹".into(),
            config_path: None,
        }
    }

    fn plan_install(&self, ctx: &InstallCtx) -> Result<Vec<ConfigEdit>> {
        let path = hooks_path(&ctx.home);
        Ok(events_to_install(ctx)
            .map(|(event, blocking)| ConfigEdit {
                path: path.clone(),
                op: EditOp::EnsureHook {
                    event: event.to_string(),
                    // Codex 的 group 不带 matcher:本机上被它接受并信任过的那份就是这个形状
                    group: json!({ "hooks": [hook_spec(ctx.port, event, blocking)] }),
                },
            })
            .collect())
    }

    fn plan_uninstall(&self, ctx: &InstallCtx) -> Result<Vec<ConfigEdit>> {
        let path = hooks_path(&ctx.home);
        Ok(managed_events()
            .map(|(event, _)| ConfigEdit {
                path: path.clone(),
                op: EditOp::RemoveOurs { event: event.to_string() },
            })
            .collect())
    }

    fn activation(&self) -> Option<&'static str> {
        Some("codex-hooks-review")
    }

    fn parse(&self, event: &str, body: &Value) -> Option<Update> {
        let state = state_of(event)?;
        let detail = match event {
            "PreToolUse" | "PostToolUse" => {
                body.get("tool_name").and_then(Value::as_str).map(str::to_string)
            }
            _ => None,
        };
        Some(Update {
            state: Some(state),
            detail,
            cwd: body.get("cwd").and_then(Value::as_str).map(str::to_string),
            title: None,
        })
    }

    fn is_blocking(&self, event: &str) -> bool {
        BLOCKING.contains(&event)
    }

    /// 与 Claude Code 同一个信封 —— 参考实现对这两家用的就是同一个 sanitizer,
    /// 而 `decision` 是**对象**这一点在 Claude Code 上已实测确认。
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
        let Some(doc) = read_hooks(&hooks_path(&ctx.home)) else {
            return (Integration::NotInstalled, Vec::new());
        };
        let mut here = 0usize;
        let mut elsewhere = 0usize;
        let mut conflicts = Vec::new();
        for (event, blocking) in managed_events() {
            for hook in hooks_of(&doc, event) {
                if is_ours(hook, ctx.port) {
                    here += 1;
                } else if is_our_shape(hook) {
                    elsewhere += 1;
                } else if blocking {
                    conflicts.push(format!("{event} 上已有别的程序的 hook"));
                }
            }
        }
        let state = if here > 0 {
            Integration::Installed
        } else if elsewhere > 0 {
            Integration::StalePort
        } else {
            Integration::NotInstalled
        };
        (state, conflicts)
    }
}

fn is_our_shape(hook: &Value) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains("http://127.0.0.1:") && c.contains("/hook/codex/"))
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
