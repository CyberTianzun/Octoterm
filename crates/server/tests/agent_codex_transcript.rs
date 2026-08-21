//! Codex 的会话记录解析。
//!
//! fixture 从**真实** rollout 文件脱敏而来:文本换占位符,结构原样保留 —— 两条并行的
//! 流、六种 payload、一条加密的 reasoning、一行坏 JSON,以及 Codex 特有的 `developer` 角色。

use octoterm_server::agent::codex_transcript::{locate, parse};
use octoterm_server::agent::transcript::{Block, Role};

const FIXTURE: &str = include_str!("fixtures/codex-rollout.jsonl");

#[test]
fn all_four_block_kinds_come_through() {
    let msgs = parse(FIXTURE, 0);
    let mut kinds = [0usize; 4];
    for m in &msgs {
        for b in &m.blocks {
            match b {
                Block::Text { .. } => kinds[0] += 1,
                Block::Thinking { .. } => kinds[1] += 1,
                Block::ToolUse { .. } => kinds[2] += 1,
                Block::ToolResult { .. } => kinds[3] += 1,
            }
        }
    }
    assert!(kinds.iter().all(|n| *n > 0), "四种块都该出现: {kinds:?}");
}

/// 思考只有渲染流(`event_msg/agent_reasoning`)里是明文;
/// `response_item/reasoning` 带的是 `encrypted_content`,必须跳过而不是当成文本塞进去。
#[test]
fn encrypted_reasoning_is_skipped_plaintext_thinking_is_kept() {
    let raw = serde_json::to_string(&parse(FIXTURE, 0)).unwrap();
    assert!(!raw.contains("ENCRYPTED"), "加密的 reasoning 漏出来了");
    assert!(raw.contains("<thinking-1>"), "明文思考没被读到");
}

/// `arguments` 是被塞进字符串里的 JSON,不先解开就会展示成一坨转义。
#[test]
fn embedded_json_arguments_are_unwrapped() {
    let found = parse(FIXTURE, 0).into_iter().flat_map(|m| m.blocks).any(|b| match b {
        Block::ToolUse { input, .. } => input.starts_with("<cmd-") && !input.contains('{'),
        _ => false,
    });
    assert!(found, "arguments 里的 JSON 没被解开");
}

/// Codex 除了 user/assistant 还有 developer 这一路,归到 System 而不是丢掉。
#[test]
fn the_developer_role_becomes_system() {
    assert!(FIXTURE.contains(r#""role":"developer""#) || FIXTURE.contains(r#""role": "developer""#));
    assert!(parse(FIXTURE, 0).iter().any(|m| matches!(m.role, Role::System)));
}

/// 工具输出归到 user 一侧 —— 协议层面它就是用户轮的内容,Claude 的记录也这么放。
#[test]
fn tool_output_is_attributed_to_the_user_side() {
    let m = parse(FIXTURE, 0)
        .into_iter()
        .find(|m| m.blocks.iter().any(|b| matches!(b, Block::ToolResult { .. })))
        .expect("应当有工具输出");
    assert!(matches!(m.role, Role::User));
}

#[test]
fn non_conversation_records_are_skipped() {
    let raw = serde_json::to_string(&parse(FIXTURE, 0)).unwrap();
    for noise in ["session_meta", "token_count", "task_started", "<skipped>"] {
        assert!(!raw.contains(noise), "非对话记录漏进来了: {noise}");
    }
}

#[test]
fn a_broken_line_does_not_kill_the_window() {
    assert!(FIXTURE.contains("{ broken codex line"));
    assert!(parse(FIXTURE, 0).len() > 10);
}

/// 增量去重的地基,和 Claude 那边同一条要求。
#[test]
fn ids_are_stable_across_two_reads() {
    let a: Vec<_> = parse(FIXTURE, 0).into_iter().map(|m| m.id).collect();
    let b: Vec<_> = parse(FIXTURE, 0).into_iter().map(|m| m.id).collect();
    assert_eq!(a, b);
}

/// 路径是**推导**出来的,不是「挑最新的那个」猜出来的:文件名末尾的 uuid 就是会话 id。
#[test]
fn locate_finds_the_file_by_session_id() {
    let home = tempfile::tempdir().unwrap();
    let day = home.path().join(".codex/sessions/2026/08/21");
    std::fs::create_dir_all(&day).unwrap();
    let want = day.join("rollout-2026-08-21T10-00-00-aaaa-bbbb.jsonl");
    std::fs::write(&want, "{}").unwrap();
    // 同目录下的别人不能被误认
    std::fs::write(day.join("rollout-2026-08-21T09-00-00-cccc-dddd.jsonl"), "{}").unwrap();

    assert_eq!(locate(home.path(), "aaaa-bbbb").as_deref(), Some(want.as_path()));
    assert_eq!(locate(home.path(), "no-such-session"), None);
}

#[test]
fn locate_also_looks_in_archived_sessions() {
    let home = tempfile::tempdir().unwrap();
    let arch = home.path().join(".codex/archived_sessions");
    std::fs::create_dir_all(&arch).unwrap();
    let want = arch.join("rollout-2025-01-01T00-00-00-old-uuid.jsonl");
    std::fs::write(&want, "{}").unwrap();
    assert_eq!(locate(home.path(), "old-uuid").as_deref(), Some(want.as_path()));
}

/// 目录不存在时不能 panic —— 没装过 Codex 的机器上这是常态。
#[test]
fn locate_on_a_missing_tree_is_none_not_a_panic() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(locate(home.path(), "whatever"), None);
}
