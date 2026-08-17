//! Windows Terminal 的 settings.json 不是 JSON,是 JSONC:它自己生成的默认配置
//! 里就带着 `//` 注释,用户手改后经常还留下尾逗号。`serde_json` 会直接拒绝。
//!
//! 这里把 JSONC 降级成 JSON,只做两件事:去注释、去尾逗号。**必须是字符串感知
//! 的** —— `"commandline": "wsl.exe -d Ubuntu // 备注"` 里的 `//` 是数据,
//! `"C:\\path"` 里的反斜杠会影响引号配对。按字符串搜索替换的实现会在这两处出错。

/// 返回一份可以交给 `serde_json` 的等价 JSON。输入不合法时不报错 —— 交给
/// 真正的 parser 去报,那里的错误信息更有用。
pub fn strip(src: &str) -> String {
    let no_comments = strip_comments(src);
    strip_trailing_commas(&no_comments)
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut it = src.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' => {
                out.push(c);
                // 字符串里原样照抄,只需正确处理 \" 以免提前认为字符串结束
                while let Some(c) = it.next() {
                    out.push(c);
                    if c == '\\' {
                        if let Some(n) = it.next() {
                            out.push(n);
                        }
                    } else if c == '"' {
                        break;
                    }
                }
            }
            '/' if it.peek() == Some(&'/') => {
                for c in it.by_ref() {
                    if c == '\n' {
                        out.push('\n'); // 保留行号,parser 报错时位置才对得上
                        break;
                    }
                }
            }
            '/' if it.peek() == Some(&'*') => {
                it.next();
                let mut prev = '\0';
                for c in it.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    if c == '\n' {
                        out.push('\n');
                    }
                    prev = c;
                }
                // 注释换成一个空格:`1/*x*/2` 不能变成 `12`
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

fn strip_trailing_commas(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            out.push(c);
            i += 1;
            while i < chars.len() {
                let c = chars[i];
                out.push(c);
                i += 1;
                if c == '\\' {
                    if i < chars.len() {
                        out.push(chars[i]);
                        i += 1;
                    }
                } else if c == '"' {
                    break;
                }
            }
            continue;
        }
        if c == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                i += 1; // 丢掉这个逗号,后面的空白照常输出
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn parse(s: &str) -> Value {
        serde_json::from_str(&strip(s)).expect("stripped JSONC should parse")
    }

    #[test]
    fn removes_line_and_block_comments() {
        let v = parse(
            r#"{
                // 这是注释
                "a": 1, /* 块注释 */
                "b": 2
            }"#,
        );
        assert_eq!(v["a"], 1);
        assert_eq!(v["b"], 2);
    }

    #[test]
    fn removes_trailing_commas_in_objects_and_arrays() {
        let v = parse(r#"{ "list": [1, 2, 3,], "x": 1, }"#);
        assert_eq!(v["list"].as_array().unwrap().len(), 3);
        assert_eq!(v["x"], 1);
    }

    #[test]
    fn does_not_touch_comment_like_text_inside_strings() {
        let v = parse(r#"{ "cmd": "wsl.exe -d Ubuntu // not a comment", "u": "http://x/*y*/" }"#);
        assert_eq!(v["cmd"], "wsl.exe -d Ubuntu // not a comment");
        assert_eq!(v["u"], "http://x/*y*/");
    }

    #[test]
    fn escaped_quotes_and_backslashes_do_not_break_string_tracking() {
        // 结尾的 \\ 是一个字面反斜杠,字符串在它之后正常闭合;若被当成转义引号,
        // 后面的 `// x` 就会被误判成在字符串里而留下来,导致解析失败。
        let v = parse(r#"{ "p": "C:\\path\\", "q": "he said \"hi\"" } // x"#);
        assert_eq!(v["p"], r"C:\path\");
        assert_eq!(v["q"], r#"he said "hi""#);
    }

    #[test]
    fn commas_inside_strings_survive() {
        let v = parse(r#"{ "a": "x,]" }"#);
        assert_eq!(v["a"], "x,]");
    }

    #[test]
    fn plain_json_passes_through_unchanged() {
        let src = r#"{"a":[1,2],"b":"c"}"#;
        assert_eq!(strip(src), src);
    }
}
