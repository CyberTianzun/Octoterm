#![allow(dead_code)]

use futures_util::{SinkExt, StreamExt};
use octoterm_protocol::{ClientMsg, Frame, ServerMsg, CONTROL_CHANNEL, PROTO_VERSION};
use octoterm_server::app::{serve, AppState};
use octoterm_server::config::WindowSize;
use octoterm_server::session::manager::SessionManager;
use tokio_tungstenite::tungstenite::Message;

pub async fn start_test_server(token: &str) -> String {
    start_test_server_with_cap(token, 1 << 20).await
}

pub async fn start_test_server_with_cap(token: &str, cap: usize) -> String {
    start_test_server_with(token, cap, WindowSize::default()).await
}

pub async fn start_test_server_with(token: &str, cap: usize, window_size: WindowSize) -> String {
    let addr = start_test_server_at(token, cap, window_size, Vec::new()).await;
    format!("ws://{addr}/ws")
}

/// 起服务并返回监听地址,给需要打 HTTP 端点的测试用。
/// `specs` 是 config.toml 里的 `[[launcher]]`,空表示只有内置 provider。
pub async fn start_test_server_at(
    token: &str,
    cap: usize,
    window_size: WindowSize,
    specs: Vec<octoterm_server::config::LauncherSpec>,
) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState {
        manager: SessionManager::new(cap, window_size),
        token: token.into(),
        launchers: std::sync::Arc::new(octoterm_server::launcher::providers(&specs)),
        listen_port: addr.port(),
    };
    tokio::spawn(async move { serve(listener, state).await.unwrap() });
    addr
}

pub fn control(msg: &ClientMsg) -> Message {
    Message::Binary(Frame::new(CONTROL_CHANNEL, serde_json::to_vec(msg).unwrap()).encode().into())
}

pub fn parse_server(msg: Message) -> Option<(u32, Result<ServerMsg, Vec<u8>>)> {
    let Message::Binary(data) = msg else { return None };
    let frame = Frame::decode(&data).unwrap();
    if frame.channel == CONTROL_CHANNEL {
        Some((frame.channel, Ok(serde_json::from_slice(&frame.payload).unwrap())))
    } else {
        Some((frame.channel, Err(frame.payload)))
    }
}

pub type Ws = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// 连上并完成握手,返回已认证的连接。
pub async fn connect(url: &str) -> Ws {
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws.send(control(&ClientMsg::Hello { token: "t".into(), proto: PROTO_VERSION })).await.unwrap();
    loop {
        if let Some((0, Ok(ServerMsg::HelloOk { .. }))) =
            parse_server(ws.next().await.unwrap().unwrap())
        {
            break;
        }
    }
    ws
}

/// 取下一条控制消息(跳过数据帧)。
pub async fn next_control(ws: &mut Ws) -> ServerMsg {
    loop {
        if let Some((0, Ok(m))) = parse_server(ws.next().await.unwrap().unwrap()) {
            return m;
        }
    }
}

pub async fn create_session(ws: &mut Ws, command: Option<Vec<String>>) -> u64 {
    ws.send(control(&ClientMsg::NewSession { name: None, command, cwd: None })).await.unwrap();
    match next_control(ws).await {
        ServerMsg::SessionEvent { session, .. } => session.id,
        other => panic!("unexpected: {other:?}"),
    }
}

/// 一个不会自己退出的会话,测完由 kill-session 收尾。
pub fn long_lived_cmd() -> Option<Vec<String>> {
    #[cfg(unix)]
    return Some(vec!["/bin/sh".into(), "-i".into()]);
    #[cfg(windows)]
    return None; // 默认 powershell
}

pub fn input_frame(channel: u32, bytes: &[u8]) -> Message {
    Message::Binary(Frame::new(channel, bytes.to_vec()).encode().into())
}

/// 收集 channel 上的原始字节,直到出现 needle 或超时。
pub async fn read_channel_until(ws: &mut Ws, channel: u32, needle: &str) -> String {
    let mut acc = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let timeout = std::time::Duration::from_millis(500);
        let Ok(Some(Ok(msg))) = tokio::time::timeout(timeout, ws.next()).await else {
            continue;
        };
        if let Some((ch, Err(bytes))) = parse_server(msg) {
            if ch == channel {
                acc.extend_from_slice(&bytes);
                if String::from_utf8_lossy(&acc).contains(needle) {
                    break;
                }
            }
        }
    }
    String::from_utf8_lossy(&acc).into_owned()
}
