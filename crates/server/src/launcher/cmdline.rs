//! 别人的配置文件里,命令往往是**一整行命令行字符串**,而 spawn 要的是 argv。
//!
//! 这中间的切分规则跟平台绑死,而且两边互不兼容:iTerm2 的 `Command` 是给
//! POSIX shell 看的,Windows Terminal 的 `commandline` 是给 `CommandLineToArgvW`
//! 看的。用错一套的典型后果是 `C:\Program Files\...` 被拆成两个参数。
//!
//! 这里两套都实现,并且**都是纯函数** —— 切分规则不依赖运行平台,所以在 macOS
//! 上也能测 Windows 那套,反之亦然。

use std::path::Path;

/// POSIX shell 的词法切分(单引号 / 双引号 / 反斜杠转义)。
///
/// 不做变量展开、通配符展开、也不理解 `|` `&&` 这些控制结构 —— 那是 shell 的
/// 活。带这些东西的 profile 切出来会是一串普通词,spawn 时自然失败,这比装作
/// 支持然后行为不对要好。
pub fn split_posix(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut have = false;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            c if c.is_whitespace() => {
                if have {
                    out.push(std::mem::take(&mut cur));
                    have = false;
                }
            }
            '\'' => {
                have = true;
                for c in it.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    cur.push(c);
                }
            }
            '"' => {
                have = true;
                while let Some(c) = it.next() {
                    if c == '"' {
                        break;
                    }
                    // 双引号里只有这四个字符能被反斜杠转义,其余反斜杠是字面量
                    if c == '\\' {
                        if let Some(&n) = it.peek() {
                            if matches!(n, '"' | '\\' | '$' | '`') {
                                cur.push(n);
                                it.next();
                                continue;
                            }
                        }
                    }
                    cur.push(c);
                }
            }
            '\\' => {
                have = true;
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            _ => {
                have = true;
                cur.push(c);
            }
        }
    }
    if have {
        out.push(cur);
    }
    out
}

/// `CommandLineToArgvW` 的切分规则。
///
/// 两条容易踩的规则,都在这里实现了:
/// 1. **argv[0] 的规则和其余参数不同** —— 它只认引号,反斜杠一律是字面量。所以
///    `"C:\Program Files\pwsh.exe" -nologo` 里那些 `\` 不会被当成转义。
/// 2. 其余参数里,反斜杠只在**紧跟引号**时才具有转义含义:2n 个反斜杠 + `"` =
///    n 个反斜杠并切换引号态;2n+1 个 + `"` = n 个反斜杠加一个字面引号。
pub fn split_windows(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;

    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if i < chars.len() {
        let mut arg0 = String::new();
        if chars[i] == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                arg0.push(chars[i]);
                i += 1;
            }
            i += 1; // 吃掉收尾引号(或越过末尾,下面的循环会立刻结束)
        } else {
            while i < chars.len() && !chars[i].is_whitespace() {
                arg0.push(chars[i]);
                i += 1;
            }
        }
        if !arg0.is_empty() {
            out.push(arg0);
        }
    }

    let mut cur = String::new();
    let mut in_quotes = false;
    let mut have = false;
    while i < chars.len() {
        let c = chars[i];
        if !in_quotes && c.is_whitespace() {
            if have {
                out.push(std::mem::take(&mut cur));
                have = false;
            }
            i += 1;
            continue;
        }
        have = true;
        match c {
            '\\' => {
                let mut n = 0;
                while i < chars.len() && chars[i] == '\\' {
                    n += 1;
                    i += 1;
                }
                if i < chars.len() && chars[i] == '"' {
                    for _ in 0..n / 2 {
                        cur.push('\\');
                    }
                    if n % 2 == 1 {
                        cur.push('"');
                    } else {
                        in_quotes = !in_quotes;
                    }
                    i += 1;
                } else {
                    for _ in 0..n {
                        cur.push('\\');
                    }
                }
            }
            '"' => {
                in_quotes = !in_quotes;
                i += 1;
            }
            _ => {
                cur.push(c);
                i += 1;
            }
        }
    }
    if have {
        out.push(cur);
    }
    out
}

/// 展开 `%VAR%`。查不到的变量**原样保留** —— 把它抹成空串会把
/// `%SystemRoot%\System32\cmd.exe` 变成一个看着像绝对路径的错误路径,更难排查。
pub fn expand_windows_env(s: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match lookup(name) {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            // 落单的 `%`,或者 `%%`:原样输出,别吞字符
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 展开开头的 `~` / `~/`。中间的 `~` 不动(那是普通字符),`~user` 也不处理。
pub fn expand_tilde(s: &str, home: Option<&Path>) -> String {
    let Some(home) = home else { return s.to_string() };
    if s == "~" {
        return home.to_string_lossy().into_owned();
    }
    match s.strip_prefix("~/") {
        Some(rest) => home.join(rest).to_string_lossy().into_owned(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_quotes_and_escapes() {
        assert_eq!(split_posix("ssh prod01"), ["ssh", "prod01"]);
        assert_eq!(split_posix("  ssh   prod01  "), ["ssh", "prod01"]);
        assert_eq!(split_posix(""), Vec::<String>::new());
        assert_eq!(
            split_posix(r#"/bin/zsh -c 'echo "hi there"'"#),
            ["/bin/zsh", "-c", r#"echo "hi there""#]
        );
        assert_eq!(split_posix(r"ls /My\ Documents"), ["ls", "/My Documents"]);
        assert_eq!(split_posix(r#"say "a\"b""#), ["say", r#"a"b"#]);
        // 空的引号对要产出一个空参数,而不是消失
        assert_eq!(split_posix(r#"cmd "" x"#), ["cmd", "", "x"]);
    }

    #[test]
    fn windows_argv0_does_not_treat_backslash_as_escape() {
        assert_eq!(
            split_windows(r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo"#),
            [r"C:\Program Files\PowerShell\7\pwsh.exe", "-NoLogo"]
        );
        assert_eq!(
            split_windows(r"C:\Windows\System32\cmd.exe /k echo hi"),
            [r"C:\Windows\System32\cmd.exe", "/k", "echo", "hi"]
        );
    }

    #[test]
    fn windows_backslash_quote_rules() {
        // 2n 个反斜杠 + 引号:n 个反斜杠,引号切换引号态
        assert_eq!(split_windows(r#"a b\\"c d""#), ["a", r"b\c d"]);
        // 2n+1 个:n 个反斜杠 + 一个字面引号
        assert_eq!(split_windows(r#"a b\"c"#), ["a", r#"b"c"#]);
        // 不跟引号的反斜杠是字面量
        assert_eq!(split_windows(r"a b\c\d"), ["a", r"b\c\d"]);
        assert_eq!(split_windows(""), Vec::<String>::new());
    }

    #[test]
    fn windows_env_expansion() {
        let env = |k: &str| match k {
            "SystemRoot" => Some(r"C:\Windows".to_string()),
            _ => None,
        };
        assert_eq!(
            expand_windows_env(r"%SystemRoot%\System32\cmd.exe", &env),
            r"C:\Windows\System32\cmd.exe"
        );
        // 查不到的原样保留
        assert_eq!(expand_windows_env("%NOPE%\\x", &env), "%NOPE%\\x");
        // 落单的 % 不吞后面的字符
        assert_eq!(expand_windows_env("100% sure", &env), "100% sure");
        assert_eq!(expand_windows_env("", &env), "");
    }

    #[test]
    fn tilde_expansion_only_at_the_front() {
        let home = Path::new("/Users/hiro");
        assert_eq!(expand_tilde("~", Some(home)), "/Users/hiro");
        assert_eq!(expand_tilde("~/work", Some(home)), "/Users/hiro/work");
        assert_eq!(expand_tilde("/a/~/b", Some(home)), "/a/~/b");
        assert_eq!(expand_tilde("~work", Some(home)), "~work");
        assert_eq!(expand_tilde("~/work", None), "~/work");
    }
}
