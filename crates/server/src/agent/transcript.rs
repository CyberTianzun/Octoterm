//! 聊天视图的消息模型,以及读 transcript 时与 agent 无关的那部分(窗口、游标)。
//!
//! **这里的形状是客户端中立的**(protocol.md R13):客户端不该知道 Claude 的
//! `content block` 长什么样,更不该为 Codex、Grok 各学一套。把各家方言归一化成
//! 下面这几种块,是 adapter 的活。
//!
//! 反过来也成立:**认不出的块不透传**。把 agent 的内部结构原样漏到线上,正是 R13
//! 要挡的事 —— 那会让客户端悄悄依赖上一个我们无权保证的契约。

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Block {
    Text {
        text: String,
    },
    /// 模型的思考。单独成块而不是丢掉 —— 它常常是这类 agent 最有信息量的部分,
    /// 折不折叠交给客户端决定。
    Thinking {
        text: String,
    },
    /// `input` 是**给人看的一行**,不是原始 JSON。
    ///
    /// 客户端要展示的是「它要干什么」,而原始入参可以很大(一次 Write 的 content
    /// 就能是几十 KB)。真要看细节,终端视图一直在那儿。
    ToolUse {
        name: String,
        input: String,
    },
    ToolResult {
        ok: bool,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    /// **同一条消息读两次必须得到同一个 id** —— 客户端的增量去重全靠它。
    /// 拿不到 agent 自己的 id 时,要用「内容决定」的方式合成,不能用读取序号。
    pub id: String,
    pub role: Role,
    /// unix 秒。拿不到就没有 —— 不编一个出来。
    pub ts: Option<u64>,
    pub blocks: Vec<Block>,
}

/// 首次加载时从文件末尾回看多少字节。
///
/// 挑「末尾一段」而不是整个文件:一个长会话的记录可以上百 MB,为了第一屏去读完它
/// 是没道理的。回看不到的部分不是丢了 —— 客户端要更早的可以带着游标往前翻(C2 之后)。
pub const WINDOW_BYTES: u64 = 4 * 1024 * 1024;

/// 增量一次最多读多少字节。
///
/// **增量用字节限、不用条数限**,这是有讲究的:条数超了就得丢一头,而增量里丢头
/// 就是静默丢消息。按字节切则读不完只是「这次没读完」,`more` 一置位客户端再拉一次,
/// 一条都不会少。
pub const INCREMENT_BYTES: u64 = 256 * 1024;

/// 单次返回的消息条数上界。**只作用于首次加载**(那时保留最近的即可);
/// 增量不受它约束,理由见 `INCREMENT_BYTES`。
pub const MAX_MESSAGES: usize = 200;

/// 单次响应的字节上界。
///
/// 条数上界挡不住这个:200 条里每条都塞满 8 KiB 的块,序列化出来能有好几 MB。
/// 这个缺口是拿一份真实的 10 MB 记录量出来的 —— 那次碰巧是 164 KiB,而「碰巧在
/// 范围内」不是限额。
///
/// 同样**只作用于首次加载**:从最旧的一头开始丢(首屏要的是最近的)。增量不能丢,
/// 它由 `INCREMENT_BYTES` 从输入侧限量。
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// 首次加载时把消息裁到两个上界之内:先按条数,再按序列化后的字节数。
/// 两者都是**从最旧的一头丢**。
pub fn clamp_for_reset(mut messages: Vec<Message>) -> Vec<Message> {
    if messages.len() > MAX_MESSAGES {
        messages.drain(..messages.len() - MAX_MESSAGES);
    }
    while messages.len() > 1 {
        let size = serde_json::to_string(&messages).map(|s| s.len()).unwrap_or(0);
        if size <= MAX_RESPONSE_BYTES {
            break;
        }
        // 一次丢一成,免得在一条条丢的过程中反复序列化整个列表
        let drop = (messages.len() / 10).max(1);
        messages.drain(..drop);
    }
    messages
}

/// 这一次该读文件的哪一段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub start: u64,
    pub end: u64,
    /// 整窗替换(首次加载,或旧游标已经失效)。客户端应当整段替换而不是追加。
    pub reset: bool,
    /// 这次没读完,还有剩的。客户端应当立刻带着新游标再拉一次。
    pub more: bool,
}

/// 纯逻辑:给定文件长度与上次的游标,算出这次读哪一段。不碰文件系统,可逐条断言。
///
/// 游标是 `(offset, len)`:`offset` 是上次消费到哪,`len` 是上次看到的文件长度。
/// 带上 `len` 是为了识别**文件变小或被换掉**(compact、开了新会话)—— 那时旧的
/// `offset` 指向的已经不是原来那条消息了,只能整窗重来,否则会把新文件的中段当成
/// 旧文件的续集接上去。
pub fn plan_read(file_len: u64, cursor: Option<(u64, u64)>) -> Plan {
    let stale = match cursor {
        None => true,
        Some((offset, seen_len)) => file_len < seen_len || offset > file_len,
    };
    if stale {
        let start = file_len.saturating_sub(WINDOW_BYTES);
        return Plan { start, end: file_len, reset: true, more: false };
    }
    let (offset, _) = cursor.expect("stale 分支已经处理了 None");
    let end = (offset + INCREMENT_BYTES).min(file_len);
    Plan { start: offset, end, reset: false, more: end < file_len }
}

/// 从读到的字节里切出「完整的若干行」,并告诉调用方消费了多少字节。
///
/// 两头各有一个陷阱:
///
/// - **开头**:按字节回切的窗口几乎必然落在半行上。那半行必须丢掉,否则第一条消息
///   是残的 —— 而残行解析失败之后,用户看到的现象是「少了一条」,极难归因。
///   续读的起点是上次算出来的行首,所以由 `drop_partial_head` 区分这两种情况。
/// - **结尾**:最后那半行多半是「正在被写入」的一条。这次不能算消费掉,游标要停在
///   它前面,下次连着后半截一起读。
pub fn slice_window(buf: &[u8], start_in_buf: usize, drop_partial_head: bool) -> (&[u8], u64) {
    let mut body = &buf[start_in_buf.min(buf.len())..];
    let mut skipped = 0usize;
    if drop_partial_head && start_in_buf > 0 {
        match body.iter().position(|b| *b == b'\n') {
            Some(i) => {
                skipped = i + 1;
                body = &body[skipped..];
            }
            // 整段里一个换行都没有:全是半行,什么都别要
            None => return (&body[body.len()..], body.len() as u64),
        }
    }
    let end = match body.iter().rposition(|b| *b == b'\n') {
        Some(i) => i + 1,
        None => 0,
    };
    (&body[..end], (skipped + end) as u64)
}

/// 一次读取的结果。
#[derive(Debug, Clone, Serialize)]
pub struct Window {
    pub messages: Vec<Message>,
    /// 不透明串,原样传回来即可。内部是 `消费到哪.当时文件多长`。
    pub cursor: String,
    pub reset: bool,
    pub more: bool,
}

/// 游标对客户端**不透明**:它只负责原样带回来。做成不透明是为了以后能换实现
/// (换成 inode + 偏移、或者带上文件指纹)而不必动协议。
pub fn encode_cursor(offset: u64, len: u64) -> String {
    format!("{offset}.{len}")
}

pub fn decode_cursor(s: &str) -> Option<(u64, u64)> {
    let (o, l) = s.split_once('.')?;
    Some((o.parse().ok()?, l.parse().ok()?))
}

/// 读一个窗口。`parse` 由 adapter 提供 —— 这里只管切,不管「一行是什么意思」。
pub fn read_window(
    path: &std::path::Path,
    cursor: Option<&str>,
    parse: &dyn Fn(&str) -> Vec<Message>,
) -> std::io::Result<Window> {
    use std::io::{Read, Seek, SeekFrom};

    let file_len = std::fs::metadata(path)?.len();
    let plan = plan_read(file_len, cursor.and_then(decode_cursor));

    // 没有新字节:直接回一个空窗,不去开文件
    if plan.start >= plan.end && !plan.reset {
        return Ok(Window {
            messages: Vec::new(),
            cursor: encode_cursor(plan.start, file_len),
            reset: false,
            more: false,
        });
    }

    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(plan.start))?;
    let mut buf = vec![0u8; (plan.end - plan.start) as usize];
    let n = f.read(&mut buf)?;
    buf.truncate(n);

    // 只有「跳进文件中段」时才丢开头那半行;续读的起点本来就是行首
    let (body, consumed) = slice_window(&buf, 0, plan.reset && plan.start > 0);
    // 记录里可能混进非法字节(截断的多字节字符),不能让它把整次读取变成错误
    let text = String::from_utf8_lossy(body);
    let mut messages = parse(&text);

    // 上界**只作用于首次加载**:那时保留最近的即可。增量不能丢头(会静默丢消息),
    // 它靠 INCREMENT_BYTES + `more` 来限量。
    if plan.reset {
        messages = clamp_for_reset(messages);
    }

    Ok(Window {
        messages,
        cursor: encode_cursor(plan.start + consumed, file_len),
        reset: plan.reset,
        more: plan.more,
    })
}

/// 单个块的文本上界。一次 `cat` 的输出可以是几 MB,聊天视图不需要那么多。
pub const MAX_BLOCK_BYTES: usize = 8 * 1024;

/// 截断到上界并标记。**标记是必须的** —— 悄悄截断会让人以为自己看到了全部。
pub fn clamp_text(mut s: String) -> String {
    if s.len() <= MAX_BLOCK_BYTES {
        return s;
    }
    // 不能从中间切断 UTF-8:往前退到字符边界
    let mut cut = MAX_BLOCK_BYTES;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.truncate(cut);
    s.push_str("\n…(已截断)");
    s
}

/// 把工具入参压平成给人看的一行。
///
/// 优先取那些「一眼能看懂它要干什么」的字段;都没有就退回紧凑 JSON。
pub fn flatten_tool_input(input: &serde_json::Value) -> String {
    if let Some(s) = input.as_str() {
        return clamp_text(s.to_string());
    }
    if let Some(obj) = input.as_object() {
        for key in ["command", "file_path", "path", "url", "pattern", "prompt"] {
            if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
                return clamp_text(v.to_string());
            }
        }
    }
    clamp_text(serde_json::to_string(input).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_untouched() {
        assert_eq!(clamp_text("hi".into()), "hi");
    }

    /// 截断必须留痕:悄悄砍掉会让人以为自己看到了全部。
    #[test]
    fn long_text_is_truncated_and_marked() {
        let out = clamp_text("x".repeat(MAX_BLOCK_BYTES * 2));
        assert!(out.len() < MAX_BLOCK_BYTES + 64);
        assert!(out.ends_with("(已截断)"));
    }

    /// 从中间切 UTF-8 会切出无效字节序列。
    #[test]
    fn truncation_respects_char_boundaries() {
        let out = clamp_text("中".repeat(MAX_BLOCK_BYTES));
        assert!(out.is_char_boundary(0));
        let _ = out.chars().count(); // 能遍历就说明没切坏
    }

    #[test]
    fn tool_input_prefers_the_human_readable_field() {
        let v = serde_json::json!({ "command": "ls -la", "description": "列目录" });
        assert_eq!(flatten_tool_input(&v), "ls -la");
        let v = serde_json::json!({ "file_path": "/etc/hosts" });
        assert_eq!(flatten_tool_input(&v), "/etc/hosts");
    }

    /// 认不出的形状也要给出点东西,而不是空白 —— 空白等于让人在看不见的东西上做判断。
    #[test]
    fn unknown_shape_falls_back_to_compact_json() {
        let v = serde_json::json!({ "weird": [1, 2] });
        assert_eq!(flatten_tool_input(&v), r#"{"weird":[1,2]}"#);
    }
}
