//! Codex adapter。与 Claude Code 的差异都在这里被钉死。

mod common;

use octoterm_server::agent::detect::DetectEnv;
use octoterm_server::agent::edit::{apply_to_json, is_ours, InstallCtx};
use octoterm_server::agent::{find, store::hook_token, Confidence};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const PORT: u16 = 7683;

fn ctx(home: &Path) -> InstallCtx {
    InstallCtx { home: home.to_path_buf(), port: PORT, include_blocking: true }
}

fn installed(home: &Path, base: Value) -> Value {
    let mut doc = base;
    for e in find("codex").unwrap().plan_install(&ctx(home)).unwrap() {
        apply_to_json(&mut doc, &e.op).unwrap();
    }
    doc
}

fn uninstalled(home: &Path, base: Value) -> Value {
    let mut doc = base;
    for e in find("codex").unwrap().plan_uninstall(&ctx(home)).unwrap() {
        apply_to_json(&mut doc, &e.op).unwrap();
    }
    doc
}

/// `~/.codex/config.toml` 是 Codex 自己写的,我们从不碰 —— 它存在就是硬证据。
/// 这一点和 Claude Code 相反:那边 `~/.claude/` 我们也会写,所以不能这么判。
#[test]
fn config_toml_is_hard_evidence() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join(".codex")).unwrap();
    fs::write(home.path().join(".codex").join("config.toml"), "model = \"gpt\"").unwrap();
    let d = find("codex")
        .unwrap()
        .detect(&DetectEnv { home: home.path().into(), path: None });
    assert!(d.installed);
    assert_eq!(d.confidence, Confidence::High);
    assert_eq!(d.reason, "config-file");
}

/// 只有我们写的 hooks.json,证明不了用户装过 Codex。
#[test]
fn our_hooks_file_alone_is_not_evidence() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".codex");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("hooks.json"), "{}").unwrap();
    let d = find("codex")
        .unwrap()
        .detect(&DetectEnv { home: home.path().into(), path: None });
    assert!(!d.installed);
}

/// Codex 不支持 http 型 hook,只能 command 型;而且它接受的 group **不带 matcher**。
#[test]
fn hooks_are_command_type_without_matcher() {
    let home = tempfile::tempdir().unwrap();
    let doc = installed(home.path(), json!({}));
    let group = &doc["hooks"]["PreToolUse"][0];
    assert!(group.get("matcher").is_none(), "Codex 的 group 不该带 matcher");
    let hook = &group["hooks"][0];
    assert_eq!(hook["type"], "command");
    let cmd = hook["command"].as_str().unwrap();
    assert!(cmd.contains("hook http://127.0.0.1:7683/hook/codex/pre-tool-use"), "命令串:{cmd}");
}

#[test]
fn blocking_hook_gets_a_human_sized_timeout() {
    let home = tempfile::tempdir().unwrap();
    let doc = installed(home.path(), json!({}));
    assert_eq!(doc["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"], 600);
    assert_eq!(doc["hooks"]["Stop"][0]["hooks"][0]["timeout"], 30);
}

#[test]
fn install_is_idempotent_and_uninstall_restores() {
    let home = tempfile::tempdir().unwrap();
    let user = json!({"hooks":{"Stop":[{"hooks":[{"type":"command","command":"their-own.sh"}]}]}});
    let once = installed(home.path(), user.clone());
    assert_eq!(installed(home.path(), once.clone()), once, "装两次必须收敛");
    assert_eq!(uninstalled(home.path(), once), user, "卸载必须还原");
}

/// 所有权跨两种承载是同一个概念:http 型看 url,command 型看命令串里的那个 URL。
#[test]
fn ownership_spans_both_transports() {
    let http = json!({"type":"http","url":"http://127.0.0.1:7683/hook/codex/stop"});
    let cmd = json!({"type":"command","command":"\"/opt/octoterm-server\" hook http://127.0.0.1:7683/hook/codex/stop"});
    assert!(is_ours(&http, PORT));
    assert!(is_ours(&cmd, PORT));

    for bad in [
        json!({"type":"command","command":"\"/opt/octoterm-server\" hook http://127.0.0.1:9999/hook/codex/stop"}),
        json!({"type":"command","command":"node /some/other-vendor-hook.js"}),
        json!({"type":"command","command":"curl http://evil.example/hook/codex/stop"}),
    ] {
        assert!(!is_ours(&bad, PORT), "不该认领: {bad}");
    }
}

/// hook 子命令的端到端:它就是 Codex 那边实际会被执行的东西。
#[tokio::test]
async fn hook_cli_reaches_the_server() {
    let addr = common::start_test_server_at("tok", 1 << 16, Default::default(), vec![]).await;
    let url = format!("http://127.0.0.1:{}/hook/codex/pre-tool-use", addr.port());
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_octoterm-server"))
        .args(["hook", &url])
        .env("OCTOTERM_SESSION_ID", "9")
        .env("OCTOTERM_HOOK_TOKEN", hook_token())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map(|mut c| {
            let mut stdin = c.stdin.take().unwrap();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(br#"{"session_id":"cx1","tool_name":"shell"}"#).await;
            });
            c
        })
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
    assert!(out.status.success());

    let body = reqwest::Client::new()
        .get(format!("http://{addr}/api/agents/sessions"))
        .header("Authorization", "Bearer tok")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["sessions"][0]["agent_id"], "codex");
    assert_eq!(v["sessions"][0]["agent_session_id"], "cx1");
    assert_eq!(v["sessions"][0]["state"], "working");
    assert_eq!(v["sessions"][0]["detail"], "shell");
}

/// 环境里没有那两个变量就**立刻退出、不联网** —— 「只管托管会话」这条边界的执行点
/// 在客户端这一侧,而不是等服务端拒收。
#[tokio::test]
async fn hook_cli_stays_silent_without_env() {
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_octoterm-server"))
        .args(["hook", "http://127.0.0.1:1/hook/codex/stop"])
        .env_remove("OCTOTERM_SESSION_ID")
        .env_remove("OCTOTERM_HOOK_TOKEN")
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty(), "不该有任何输出 —— 输出会被 agent 当成决策");
}

/// 宿主不在时也必须安静退出:绝不能把 agent 卡住,也绝不能打印出什么被当成决策。
#[tokio::test]
async fn hook_cli_survives_a_dead_host() {
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_octoterm-server"))
        .args(["hook", "http://127.0.0.1:1/hook/codex/stop"])
        .env("OCTOTERM_SESSION_ID", "1")
        .env("OCTOTERM_HOOK_TOKEN", "whatever")
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
}
