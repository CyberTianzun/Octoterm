//! Antigravity CLI 的会话记录 → 归一化消息。
//!
//! 文件是 `~/.gemini/antigravity-cli/brain/<会话 id>/.system_generated/logs/transcript_full.jsonl`
//! (hook payload 的 `transcriptPath` 直接给出,也支持 `transcript.jsonl`)。格式按 agy 1.3.5 / 1.4.5 的实际文件
//! (见 `tests/fixtures/antigravity-transcript.jsonl`):每行是一个**步骤**,
//! `{step_index, source, type, status, created_at, content?, thinking?, tool_calls?, error?}`。
//!
//! - `USER_INPUT`:用户消息。正文包在 `<USER_REQUEST>` 里,后面跟着 agy 自己加的
//!   元数据(本地时间、设置变更),只取标签里面的。
//! - `PLANNER_RESPONSE`:模型这一步的输出 —— `content` 是正文(可能为空),
//!   `tool_calls` 是这一步发起的工具调用,新版的 `thinking` 作为独立思考块读取。
//! - `GENERIC`:工具执行结果。`DONE` 成功,`ERROR` 失败、原因在 `error`。
//!   `content` 开头有两行 `Created At:` / `Completed At:`,是元数据,去掉。
//! - 认不出的步骤类型跳过,不猜。
//!
//! id 取 `step_index`:同一步读几次都是同一个 id。

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
    let step: Value = serde_json::from_str(line).ok()?;
    let content = step.get("content").and_then(Value::as_str).unwrap_or_default();
    let (role, blocks) = match step.get("type").and_then(Value::as_str)? {
        "USER_INPUT" => {
            let text = user_request(content).trim().to_string();
            (Role::User, if text.is_empty() { Vec::new() } else { vec![Block::Text { text: clamp_text(text) }] })
        }
        "PLANNER_RESPONSE" => {
            let mut blocks = Vec::new();
            if let Some(thinking) = step.get("thinking").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
                blocks.push(Block::Thinking { text: clamp_text(thinking.trim().to_string()) });
            }
            if !content.trim().is_empty() {
                blocks.push(Block::Text { text: clamp_text(content.trim().to_string()) });
            }
            for call in step.get("tool_calls").and_then(Value::as_array).into_iter().flatten() {
                let name = call.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                let args = call.get("args").cloned().unwrap_or(Value::Null);
                blocks.push(Block::ToolUse { name, input: flatten_tool_input(&args) });
            }
            (Role::Assistant, blocks)
        }
        // 工具输出归到 user 一侧,和 Claude、Codex、Grok 的放法一致
        "GENERIC" => {
            if step.get("status").and_then(Value::as_str) == Some("RUNNING") { return None; }
            let ok = step.get("status").and_then(Value::as_str) != Some("ERROR");
            let text = match step.get("error").and_then(Value::as_str) {
                Some(e) if !ok && !e.is_empty() => e.to_string(),
                _ => strip_step_header(content).to_string(),
            };
            (Role::User, vec![Block::ToolResult { ok, text: clamp_text(text) }])
        }
        _ => return None,
    };
    if blocks.is_empty() {
        return None;
    }
    let id = step
        .get("step_index")
        .and_then(Value::as_u64)
        .map(|i| format!("step-{i}"))
        .unwrap_or_else(|| offset_id(at));
    let ts = step.get("created_at").and_then(Value::as_str).and_then(unix_from_rfc3339);
    Some(Message { id, role, ts, blocks })
}

/// `<USER_REQUEST>…</USER_REQUEST>` 里的部分;没有这对标签就整段都是。
fn user_request(content: &str) -> &str {
    let Some(start) = content.find("<USER_REQUEST>") else { return content };
    let body = &content[start + "<USER_REQUEST>".len()..];
    body.find("</USER_REQUEST>").map_or(body, |end| &body[..end])
}

/// 去掉开头的 `Created At:` / `Completed At:` 元数据行。
fn strip_step_header(content: &str) -> &str {
    let mut rest = content;
    while rest.starts_with("Created At:") || rest.starts_with("Completed At:") {
        rest = rest.split_once('\n').map_or("", |(_, tail)| tail);
    }
    rest
}

/// `2026-10-15T11:05:23Z` → unix 秒。只认 agy 写的这种 UTC 形状(可带小数秒),
/// 别的形状返回 `None` —— 时间戳是锦上添花,宁缺勿错。
fn unix_from_rfc3339(s: &str) -> Option<u64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.splitn(3, '-').map(str::parse::<i64>);
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let time = time.split('.').next()?;
    let mut t = time.splitn(3, ':').map(str::parse::<i64>);
    let (hh, mm, ss) = (t.next()?.ok()?, t.next()?.ok()?, t.next()?.ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Howard Hinnant 的 days_from_civil
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

/// 由会话 id 找到记录文件。hook 事件里通常直接带着 `transcriptPath`,这里是它缺席时的退路。
pub fn locate(home: &std::path::Path, session_id: &str) -> Option<std::path::PathBuf> {
    // 会话 id 会拼进路径,不许它带分隔符跳出 brain 目录
    if session_id.is_empty() || session_id.contains(['/', '\\']) || session_id.contains("..") {
        return None;
    }
    let logs = home
        .join(".gemini")
        .join("antigravity-cli")
        .join("brain")
        .join(session_id)
        .join(".system_generated")
        .join("logs");
    ["transcript_full.jsonl", "transcript.jsonl"].into_iter()
        .map(|name| logs.join(name)).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_utc() {
        assert_eq!(unix_from_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_from_rfc3339("2026-10-15T11:05:23Z"), Some(1_792_062_323));
        assert_eq!(unix_from_rfc3339("2000-02-29T12:00:00.5Z"), Some(951_825_600));
        assert_eq!(unix_from_rfc3339("2026-10-15T11:05:23+08:00"), None);
        assert_eq!(unix_from_rfc3339("garbage"), None);
    }

    #[test]
    fn user_request_unwraps_and_drops_metadata() {
        let c = "<USER_REQUEST>\nhello\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nx\n</ADDITIONAL_METADATA>";
        assert_eq!(user_request(c).trim(), "hello");
        assert_eq!(user_request("plain"), "plain");
    }

    #[test]
    fn step_header_is_stripped() {
        assert_eq!(strip_step_header("Created At: a\nCompleted At: b\nbody\nmore"), "body\nmore");
        assert_eq!(strip_step_header("no header"), "no header");
    }
}
