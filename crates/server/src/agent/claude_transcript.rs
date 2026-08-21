//! Claude Code 的 transcript(JSONL)→ 归一化消息。
//!
//! 输入是**一整个窗口的文本**,输出是消息。窗口怎么切、游标怎么走,是
//! [`super::transcript`] 的事 —— 那部分对三家 agent 是同一套,只有「一行是什么意思」
//! 属于这里。
//!
//! 两条硬规则:
//!
//! 1. **单行失败只跳过那一行**。窗口是按字节切的,首尾很可能是半行;一个坏字节
//!    不该让整段对话变成「读不了」。
//! 2. **同一条消息读两次必须得到同一个 id**。客户端的增量去重全靠它,所以 id 只能
//!    从内容里来(记录自带的 `uuid`,没有就按内容哈希),**绝不能用读取序号** ——
//!    窗口起点会变,序号跟着变,客户端就会把同一条消息渲染两遍。

use serde_json::Value;

use super::transcript::{clamp_text, flatten_tool_input, Block, Message, Role};

pub fn parse(text: &str) -> Vec<Message> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Message> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    // 坏行(半行、截断、格式变了)直接跳过,不影响窗口里的其它行
    let row: Value = serde_json::from_str(line).ok()?;

    let role = match row.get("type").and_then(Value::as_str)? {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        // system / attachment / queue-operation / file-history-* 等等都不是对话内容。
        // **不认识的记录类型一律跳过**,而不是尽力猜 —— 猜错会把内部结构漏给客户端。
        _ => return None,
    };

    let message = row.get("message")?;
    let blocks = match message.get("content") {
        Some(Value::String(s)) => vec![Block::Text { text: clamp_text(s.clone()) }],
        Some(Value::Array(items)) => items.iter().filter_map(parse_block).collect(),
        _ => return None,
    };
    if blocks.is_empty() {
        return None;
    }

    Some(Message {
        id: message_id(&row, &blocks),
        role,
        ts: row.get("timestamp").and_then(Value::as_str).and_then(parse_ts),
        blocks,
    })
}

fn parse_block(b: &Value) -> Option<Block> {
    Some(match b.get("type").and_then(Value::as_str)? {
        "text" => Block::Text { text: clamp_text(str_of(b, "text")) },
        "thinking" => Block::Thinking { text: clamp_text(str_of(b, "thinking")) },
        "tool_use" => Block::ToolUse {
            name: b.get("name").and_then(Value::as_str).unwrap_or("?").to_string(),
            input: flatten_tool_input(b.get("input").unwrap_or(&Value::Null)),
        },
        "tool_result" => Block::ToolResult {
            ok: !b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            text: clamp_text(result_text(b.get("content"))),
        },
        // 认不出的块**丢掉,不透传**(R13)。agent 加了新块类型是常事,把它原样漏到
        // 线上会让客户端悄悄依赖一个我们无权保证的契约。
        _ => return None,
    })
}

fn str_of(b: &Value, key: &str) -> String {
    b.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// `tool_result.content` 可能是字符串,也可能是一组块。
fn result_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|x| x.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
        None => String::new(),
    }
}

/// 稳定 id。优先用记录自带的 `uuid`;没有就按**内容**哈希 —— 绝不用读取序号。
fn message_id(row: &Value, blocks: &[Block]) -> String {
    for key in ["uuid", "id"] {
        if let Some(v) = row.get(key).and_then(Value::as_str) {
            return v.to_string();
        }
    }
    if let Some(v) = row.get("message").and_then(|m| m.get("id")).and_then(Value::as_str) {
        return v.to_string();
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in blocks {
        for byte in serde_json::to_string(b).unwrap_or_default().bytes() {
            h ^= byte as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("h{h:016x}")
}

/// ISO 8601 → unix 秒。**不引入日期库**:只认 `YYYY-MM-DDTHH:MM:SS` 这个前缀,
/// 认不出就返回 `None`(模型里 `ts` 本来就是可选的,不编一个出来)。
fn parse_ts(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let n = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let (h, mi, se) = (n(11, 13)?, n(14, 16)?, n(17, 19)?);
    // 民用历法转儒略日,再折算成 unix 秒(纯整数运算,没有闰秒概念)
    let a = (14 - mo) / 12;
    let yy = y + 4800 - a;
    let mm = mo + 12 * a - 3;
    let jdn = d + (153 * mm + 2) / 5 + 365 * yy + yy / 4 - yy / 100 + yy / 400 - 32045;
    let secs = (jdn - 2_440_588) * 86_400 + h * 3600 + mi * 60 + se;
    u64::try_from(secs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_a_known_date_round_trip() {
        assert_eq!(parse_ts("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_ts("2026-08-21T00:00:00.000Z"), Some(1_787_270_400));
        assert_eq!(parse_ts("not a timestamp"), None);
    }
}
