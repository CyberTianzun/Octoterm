//! Codex 的会话记录(`rollout-*.jsonl`)→ 归一化消息。
//!
//! 与 Claude 的记录有三处不同,都是实测出来的:
//!
//! 1. **一个文件里有两条并行的流**。`response_item` 是 API 层(消息、工具调用及其
//!    输出),`event_msg` 是渲染层(思考、token 计数、任务状态)。两条流在同一个文件里
//!    按时间交错,所以只要**每种记录只映射一次**,顺序天然是对的。
//! 2. **思考只在渲染流里是明文**。`response_item/reasoning` 带的是 `encrypted_content`,
//!    读不了;`event_msg/agent_reasoning` 的 `text` 是明文。所以思考取后者。
//! 3. **多数记录没有自带 id**,得按内容合成(见 `transcript::synth_id`)。
//!
//! 取 `response_item` 而不是 `event_msg` 来做消息与工具,是因为前者结构完整:
//! `event_msg` 里没有通用的工具调用记录。两边都取会重复。

use serde_json::Value;

use super::transcript::{clamp_text, flatten_tool_input, offset_id, Block, Message, Role};

pub fn parse(text: &str, base: u64) -> Vec<Message> {
    let mut at = base;
    let mut out = Vec::new();
    for line in text.split_inclusive('\n') {
        if let Some(m) = parse_line(line, at) {
            out.push(m);
        }
        at += line.len() as u64;
    }
    out
}

fn parse_line(line: &str, at: u64) -> Option<Message> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let row: Value = serde_json::from_str(line).ok()?;
    let payload = row.get("payload")?;
    let kind = payload.get("type").and_then(Value::as_str)?;

    let (role, blocks) = match (row.get("type").and_then(Value::as_str)?, kind) {
        ("response_item", "message") => (
            role_of(payload.get("role").and_then(Value::as_str)),
            content_texts(payload.get("content")),
        ),
        ("response_item", "function_call") => (
            Role::Assistant,
            vec![Block::ToolUse {
                name: str_of(payload, "name"),
                // `arguments` 是一个 **JSON 字符串**,不是对象 —— 先解出来再压平,
                // 否则展示的是一坨转义
                input: flatten_tool_input(&parse_embedded(payload.get("arguments"))),
            }],
        ),
        ("response_item", "custom_tool_call") => (
            Role::Assistant,
            vec![Block::ToolUse {
                name: str_of(payload, "name"),
                input: clamp_text(str_of(payload, "input")),
            }],
        ),
        // 工具输出归到 user 一侧:协议层面它就是用户轮的内容,Claude 的记录也是这么放的
        ("response_item", "function_call_output") => (
            Role::User,
            vec![Block::ToolResult { ok: true, text: clamp_text(str_of(payload, "output")) }],
        ),
        ("response_item", "custom_tool_call_output") => (
            Role::User,
            vec![Block::ToolResult {
                ok: true,
                text: clamp_text(join_texts(payload.get("output"))),
            }],
        ),
        // 思考只有这里是明文;response_item/reasoning 是加密的,跳过
        ("event_msg", "agent_reasoning") => (
            Role::Assistant,
            vec![Block::Thinking { text: clamp_text(str_of(payload, "text")) }],
        ),
        // session_meta / turn_context / token_count / task_started / world_state …
        // 都不是对话内容。**认不出的一律跳过**,不猜。
        _ => return None,
    };

    if blocks.is_empty() {
        return None;
    }
    // Codex 的记录多数不带自己的 id,而它的内容重复率很高(同一个 wait 调用、
    // 一样的短输出),内容哈希会撞 —— 撞了客户端就会把不同的消息当成重复丢掉。
    let id = payload
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| offset_id(at));
    Some(Message { id, role, ts: None, blocks })
}

/// Codex 除了 user/assistant 还有 `developer`(系统指令那一路)。归到 System。
fn role_of(role: Option<&str>) -> Role {
    match role {
        Some("user") => Role::User,
        Some("assistant") => Role::Assistant,
        _ => Role::System,
    }
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// `arguments` 这类字段是被塞进字符串里的 JSON。解得开就用解出来的,解不开就当纯文本。
fn parse_embedded(v: Option<&Value>) -> Value {
    match v.and_then(Value::as_str) {
        Some(s) => serde_json::from_str(s).unwrap_or(Value::String(s.to_string())),
        None => v.cloned().unwrap_or(Value::Null),
    }
}

fn content_texts(v: Option<&Value>) -> Vec<Block> {
    v.and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .filter(|t| !t.is_empty())
                .map(|t| Block::Text { text: clamp_text(t.to_string()) })
                .collect()
        })
        .unwrap_or_default()
}

fn join_texts(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|x| x.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// 由会话 id 找到记录文件。
///
/// Codex 的 hook payload **不带路径**(实测:二进制里没有这类字段),但文件名末尾的
/// uuid 就是 `session_meta.id`,所以能确定性地推出来 —— 不是靠「挑最新的那个」去猜。
///
/// 布局是 `sessions/<年>/<月>/<日>/rollout-<时间戳>-<uuid>.jsonl`,归档的在
/// `archived_sessions/` 平铺。两处都找,深度写死,不做无界递归。
pub fn locate(home: &std::path::Path, session_id: &str) -> Option<std::path::PathBuf> {
    let suffix = format!("-{session_id}.jsonl");
    let root = home.join(".codex");
    if let Some(p) = find_in(&root.join("archived_sessions"), &suffix, 0) {
        return Some(p);
    }
    find_in(&root.join("sessions"), &suffix, 3)
}

fn find_in(dir: &std::path::Path, suffix: &str, depth: usize) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut dirs = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            dirs.push(path);
        } else if path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(suffix)) {
            return Some(path);
        }
    }
    if depth == 0 {
        return None;
    }
    // 新的排在后面,倒着找更快命中
    dirs.sort();
    dirs.into_iter().rev().find_map(|d| find_in(&d, suffix, depth - 1))
}
