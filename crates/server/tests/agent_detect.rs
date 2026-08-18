//! agent 检测:必须是「三元证据」,且不能被一个空目录骗过。
//!
//! 这里每一条都对应 clawd-on-desk 踩过的坑:它自己会创建 `~/.claude/`,于是
//! 「目录存在」变成了自证,最后不得不把 claude-code 整个从默认检测里排除。

use octoterm_server::agent::detect::DetectEnv;
use octoterm_server::agent::{find, Confidence};
use std::fs;
use std::path::Path;

/// 造一个只有 home、PATH 为空的环境 —— PATH 必须显式注入,否则单测会读到
/// 开发机上真实存在的 `claude`,把「没装」的用例全部染成「装了」。
fn env(home: &Path) -> DetectEnv {
    DetectEnv { home: home.to_path_buf(), path: None }
}

fn detect(home: &Path) -> octoterm_server::agent::Detected {
    find("claude-code").unwrap().detect(&env(home))
}

#[test]
fn empty_dir_is_not_installed() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir(home.path().join(".claude")).unwrap();
    let d = detect(home.path());
    assert!(!d.installed, "只有一个空目录不足以判定已安装,reason={}", d.reason);
}

#[test]
fn missing_home_is_not_installed() {
    let home = tempfile::tempdir().unwrap();
    let d = detect(home.path());
    assert!(!d.installed);
    assert_eq!(d.reason, "not-found");
}

#[test]
fn settings_with_user_content_is_high_confidence() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".claude");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("settings.json"), r#"{"model":"opus"}"#).unwrap();
    let d = detect(home.path());
    assert!(d.installed);
    assert_eq!(d.confidence, Confidence::High);
    assert_eq!(d.reason, "config-file");
}

/// 配置里只有我们自己写进去的 hook —— 这只能证明 octoterm 来过,不能证明
/// 用户装了 Claude Code。
#[test]
fn settings_with_only_our_hooks_is_not_evidence() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".claude");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("settings.json"),
        r#"{"hooks":{"Stop":[{"matcher":"","hooks":[
            {"type":"http","url":"http://127.0.0.1:7683/hook/claude-code/stop"}]}]}}"#,
    )
    .unwrap();
    let d = detect(home.path());
    assert!(!d.installed, "只有我方 hook 的配置不构成证据");
}

/// 但只要 hooks 里混着一条别人的,就说明用户自己配过。
#[test]
fn settings_with_foreign_hook_is_evidence() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".claude");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("settings.json"),
        r#"{"hooks":{"Stop":[{"matcher":"","hooks":[
            {"type":"command","command":"my-own.sh"}]}]}}"#,
    )
    .unwrap();
    assert!(detect(home.path()).installed);
}

#[test]
fn cli_on_path_is_high_confidence() {
    let home = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let exe = bin.path().join(if cfg!(windows) { "claude.cmd" } else { "claude" });
    fs::write(&exe, "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let e = DetectEnv { home: home.path().to_path_buf(), path: Some(bin.path().into()) };
    let d = find("claude-code").unwrap().detect(&e);
    assert!(d.installed);
    assert_eq!(d.reason, "cli-path");
}

/// 目录里有 settings.json 以外的东西 → 中置信。用户跑过 Claude Code 才会有
/// 这些(history、projects 之类)。
#[test]
fn dir_with_other_entries_is_medium() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".claude");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("history.jsonl"), "{}").unwrap();
    let d = detect(home.path());
    assert!(d.installed);
    assert_eq!(d.confidence, Confidence::Medium);
    assert_eq!(d.reason, "parent-dir");
}

/// 坏掉的配置文件不该让扫描整个失败 —— 局部失败原则,和 launcher 的 provider 一致。
#[test]
fn broken_settings_does_not_panic() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".claude");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("settings.json"), "{ this is not json").unwrap();
    let _ = detect(home.path());
}
