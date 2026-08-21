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
