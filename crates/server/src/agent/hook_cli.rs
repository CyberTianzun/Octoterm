//! `octoterm-server hook <url>` —— 给只支持 `type: "command"` 的 agent 用的 hook 客户端。
//!
//! 为什么需要它:Claude Code 支持 `type: "http"`,agent 直接 POST 到 server,中间什么都
//! 不需要。**Codex 不支持** —— 它的 `HookHandlerConfig` 只有 command / prompt / agent
//! 三种(从 codex 二进制里的类型名读出来的),所以必须有一个可执行文件站在中间。
//!
//! 那个可执行文件就是 octoterm 自己。不引入 HTTP 客户端依赖:目标永远是 127.0.0.1,
//! 请求形状完全固定,手写一个几十行的 HTTP/1.1 POST 比拖进一个通用客户端更符合
//! 「单个小体积静态二进制」的定位。
//!
//! 行为约定(顺序就是防线的顺序):
//!
//! 1. 环境里没有 `OCTOTERM_SESSION_ID` / `OCTOTERM_HOOK_TOKEN` → **立刻退出,不联网**。
//!    这是「只管托管会话」这条边界的执行点:在 octoterm 之外启动的 agent 拿不到这两个
//!    变量,于是连一个包都不会发出去。
//! 2. 任何失败都**静默 exit 0 且不打印** —— 不打印就等于「无决定」,agent 回落到它自己
//!    的审批流程。宿主不在、端口换了、网络抽风,都绝不能把 agent 卡住或者替它决定。

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// stdin 的上界。`tool_input` 可能带很长的命令原文,但 512 KiB 之外的东西对决策没有
/// 帮助,超了就当读不到 —— 与服务端 `/hook/*` 的 body 上限一致。
const MAX_STDIN: usize = 512 * 1024;

/// 决策类 hook 那头最长等 600 秒,我们比它短一点收手,好让「无决定」由我们写出来。
const READ_TIMEOUT: Duration = Duration::from_secs(590);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// 返回进程退出码。永远是 0 —— 见模块文档第 2 条。
pub fn run(url: &str) -> i32 {
    let (Ok(session), Ok(token)) =
        (std::env::var("OCTOTERM_SESSION_ID"), std::env::var("OCTOTERM_HOOK_TOKEN"))
    else {
        return 0;
    };
    if session.is_empty() || token.is_empty() {
        return 0;
    }

    let mut body = Vec::new();
    let mut stdin = std::io::stdin().take(MAX_STDIN as u64);
    if stdin.read_to_end(&mut body).is_err() {
        return 0;
    }

    match post(url, &session, &token, &body) {
        // 原样透传响应体:决策的方言由服务端的 adapter 渲染,这里不解释它
        Ok(reply) if !reply.is_empty() => {
            let _ = std::io::stdout().write_all(&reply);
        }
        _ => {}
    }
    0
}

/// `http://127.0.0.1:<port>/<path>` → `(host_port, path)`。只认回环,别的一律不发。
fn split_url(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("http://127.0.0.1:")?;
    let (port, path) = rest.split_once('/')?;
    port.parse::<u16>().ok()?;
    Some((format!("127.0.0.1:{port}"), format!("/{path}")))
}

fn post(url: &str, session: &str, token: &str, body: &[u8]) -> std::io::Result<Vec<u8>> {
    let (addr, path) = split_url(url)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad url"))?;
    let sock = addr
        .parse()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad addr"))?;
    let mut stream = TcpStream::connect_timeout(&sock, CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;

    let head = format!(
        "POST {path} HTTP/1.1\r\n\
         Host: {addr}\r\n\
         Authorization: Bearer {token}\r\n\
         X-Octoterm-Session: {session}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\r\n",
        len = body.len(),
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    Ok(response_body(&raw))
}

/// 从原始响应里取出 body。
///
/// 只处理我们自己的服务端会产生的形状:`Connection: close` + 明确的
/// `Content-Length`,没有分块编码。**非 2xx 一律当作空 body** —— 那对 agent 就是
/// 「无决定」,而不是把错误页当成决策打出去。
pub fn response_body(raw: &[u8]) -> Vec<u8> {
    let Some(head_end) = find(raw, b"\r\n\r\n") else { return Vec::new() };
    let head = String::from_utf8_lossy(&raw[..head_end]);
    let status_ok = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .is_some_and(|c| (200..300).contains(&c));
    if !status_ok {
        return Vec::new();
    }
    raw[head_end + 4..].to_vec()
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_body_from_a_normal_response() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(response_body(raw), b"{}");
    }

    /// 非 2xx 当作无决定,绝不把错误页当决策打给 agent。
    #[test]
    fn non_2xx_yields_no_output() {
        let raw = b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 12\r\n\r\nunauthorized";
        assert!(response_body(raw).is_empty());
    }

    #[test]
    fn truncated_response_yields_no_output() {
        assert!(response_body(b"HTTP/1.1 200 OK").is_empty());
    }

    #[test]
    fn only_loopback_urls_are_accepted() {
        assert!(split_url("http://127.0.0.1:7683/hook/codex/stop").is_some());
        for bad in [
            "https://127.0.0.1:7683/hook/codex/stop",
            "http://10.0.0.1:7683/hook/codex/stop",
            "http://127.0.0.1:notaport/hook/codex/stop",
            "http://127.0.0.1:7683",
        ] {
            assert!(split_url(bad).is_none(), "不该接受: {bad}");
        }
    }
}
