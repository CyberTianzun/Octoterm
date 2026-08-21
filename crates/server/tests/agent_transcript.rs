//! Claude transcript 的解析。
//!
//! fixture 是从**真实会话**脱敏来的:文本换成占位符,结构原样保留(四种块、
//! 三种要跳过的记录类型、一个未来才有的未知块、一行坏 JSON)。合成数据在这里没有
//! 价值 —— 真实风险全在「真文件里到底长什么样」,合成的只验证我对格式的想象。

use octoterm_server::agent::claude_transcript::parse;
use octoterm_server::agent::transcript::{Block, Role, MAX_BLOCK_BYTES};

const FIXTURE: &str = include_str!("fixtures/claude-transcript.jsonl");

fn parsed() -> Vec<octoterm_server::agent::transcript::Message> {
    parse(FIXTURE)
}

#[test]
fn four_block_kinds_are_normalized() {
    let msgs = parsed();
    let mut text = 0;
    let mut thinking = 0;
    let mut tool_use = 0;
    let mut tool_result = 0;
    for m in &msgs {
        for b in &m.blocks {
            match b {
                Block::Text { .. } => text += 1,
                Block::Thinking { .. } => thinking += 1,
                Block::ToolUse { .. } => tool_use += 1,
                Block::ToolResult { .. } => tool_result += 1,
            }
        }
    }
    assert!(text > 0 && thinking > 0 && tool_use > 0 && tool_result > 0,
        "四种块都该出现: text={text} thinking={thinking} tool_use={tool_use} tool_result={tool_result}");
}

/// `system` / `attachment` / `queue-operation` 都不是对话内容。
#[test]
fn non_message_records_are_skipped() {
    for m in parsed() {
        assert!(matches!(m.role, Role::User | Role::Assistant), "混进了非对话记录");
    }
}

/// agent 加新块类型是常事。**丢掉,不透传**(R13)—— 原样漏出去会让客户端悄悄
/// 依赖一个我们无权保证的契约。
#[test]
fn unknown_block_kinds_are_dropped_not_leaked() {
    let raw = serde_json::to_string(&parsed()).unwrap();
    assert!(!raw.contains("some_future_block"), "未知块类型漏到线上了");
    // 但同一条消息里它后面那个正常块必须还在 —— 丢的是块,不是整条消息
    assert!(raw.contains("<text-after-unknown>"), "未知块把同一条消息里的正常块也带走了");
}

/// 窗口是按字节切的,首尾很可能是半行。一个坏字节不该让整段对话变成「读不了」。
#[test]
fn a_broken_line_does_not_kill_the_window() {
    assert!(FIXTURE.contains("{ this line is broken json"), "fixture 里得有一行坏的");
    assert!(parsed().len() > 5, "一行坏 JSON 把整个窗口毁了");
}

#[test]
fn tool_input_is_flattened_to_one_line() {
    let found = parsed().iter().flat_map(|m| m.blocks.clone()).any(|b| match b {
        Block::ToolUse { input, .. } => !input.contains('\n'),
        _ => false,
    });
    assert!(found, "工具入参应当被压成一行");
}

/// **增量去重的地基**:同一条消息读两次必须得到同一个 id。
/// 用读取序号做 id 的话,窗口起点一变客户端就会重复渲染。
#[test]
fn ids_are_stable_across_two_reads() {
    let a: Vec<_> = parsed().into_iter().map(|m| m.id).collect();
    let b: Vec<_> = parsed().into_iter().map(|m| m.id).collect();
    assert_eq!(a, b);
    // 从中间某行开始的窗口,重叠部分的 id 必须和整份读出来的一致
    let half = FIXTURE.lines().skip(FIXTURE.lines().count() / 2).collect::<Vec<_>>().join("\n");
    let tail: Vec<_> = parse(&half).into_iter().map(|m| m.id).collect();
    assert!(tail.iter().all(|id| a.contains(id)), "换个窗口起点,id 就变了");
}

#[test]
fn oversized_block_is_truncated_and_marked() {
    let huge = format!(
        r#"{{"type":"assistant","uuid":"big","message":{{"role":"assistant","content":[{{"type":"text","text":"{}"}}]}}}}"#,
        "x".repeat(MAX_BLOCK_BYTES * 2)
    );
    let m = parse(&huge);
    assert_eq!(m.len(), 1);
    match &m[0].blocks[0] {
        Block::Text { text } => {
            assert!(text.len() < MAX_BLOCK_BYTES + 64);
            assert!(text.ends_with("(已截断)"), "截断必须留痕");
        }
        other => panic!("形状不对: {other:?}"),
    }
}

#[test]
fn tool_result_error_is_marked() {
    let line = r#"{"type":"user","uuid":"e1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"boom","is_error":true}]}}"#;
    match &parse(line)[0].blocks[0] {
        Block::ToolResult { ok, .. } => assert!(!ok, "报错的工具结果没被标出来"),
        other => panic!("形状不对: {other:?}"),
    }
}

/// 纯字符串形态的 content(用户消息常见)也要认。
#[test]
fn plain_string_content_becomes_a_text_block() {
    let line = r#"{"type":"user","uuid":"p1","message":{"role":"user","content":"你好"}}"#;
    match &parse(line)[0].blocks[0] {
        Block::Text { text } => assert_eq!(text, "你好"),
        other => panic!("形状不对: {other:?}"),
    }
}
