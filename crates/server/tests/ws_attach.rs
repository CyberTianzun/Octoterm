mod common;
use common::{control, parse_server, start_test_server};
use futures_util::{SinkExt, StreamExt};
use octoterm_protocol::{AttachMode, ClientMsg, Frame, ServerMsg, PROTO_VERSION};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

async fn connect(url: &str) -> Ws {
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

/// 收集 channel 上的原始字节,直到出现 needle 或超时
async fn read_channel_until(ws: &mut Ws, channel: u32, needle: &str) -> String {
    let mut acc = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_millis(500), ws.next()).await
        else {
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

async fn next_control(ws: &mut Ws) -> ServerMsg {
    loop {
        if let Some((0, Ok(m))) = parse_server(ws.next().await.unwrap().unwrap()) {
            return m;
        }
    }
}

fn long_lived_cmd() -> Option<Vec<String>> {
    #[cfg(unix)]
    return Some(vec!["/bin/sh".into(), "-i".into()]);
    #[cfg(windows)]
    return None; // 默认 powershell
}

async fn create_session(ws: &mut Ws, command: Option<Vec<String>>) -> u64 {
    ws.send(control(&ClientMsg::NewSession { name: None, command })).await.unwrap();
    match next_control(ws).await {
        ServerMsg::SessionEvent { session, .. } => session.id,
        other => panic!("unexpected: {other:?}"),
    }
}

fn input_frame(channel: u32, bytes: &[u8]) -> Message {
    Message::Binary(Frame::new(channel, bytes.to_vec()).encode().into())
}

#[tokio::test]
async fn attach_echo_roundtrip() {
    let url = start_test_server("t").await;
    let mut ws = connect(&url).await;
    let id = create_session(&mut ws, long_lived_cmd()).await;

    ws.send(control(&ClientMsg::Attach { id, channel: 1, last_seq: None, cols: 100, rows: 30 }))
        .await
        .unwrap();
    match next_control(&mut ws).await {
        ServerMsg::Attached { channel: 1, mode: AttachMode::Resync, .. } => {}
        other => panic!("unexpected: {other:?}"),
    }
    // resync 流程:begin → 重绘帧 → end
    assert!(matches!(next_control(&mut ws).await, ServerMsg::ResyncBegin { channel: 1 }));

    tokio::time::sleep(Duration::from_millis(300)).await;
    ws.send(input_frame(1, b"echo ECHO_MARK\r")).await.unwrap();
    let got = read_channel_until(&mut ws, 1, "ECHO_MARK").await;
    assert!(got.contains("ECHO_MARK"), "got: {got}");

    ws.send(control(&ClientMsg::KillSession { id })).await.unwrap();
    // 退出通知
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(tokio::time::Instant::now() < deadline);
        match tokio::time::timeout(Duration::from_secs(1), ws.next()).await {
            Ok(Some(Ok(msg))) => {
                if let Some((0, Ok(ServerMsg::SessionExited { channel: 1, .. }))) = parse_server(msg) {
                    break;
                }
            }
            _ => continue,
        }
    }
}

#[tokio::test]
async fn reconnect_replays_missed_bytes() {
    let url = start_test_server("t").await;
    let mut ws = connect(&url).await;
    let id = create_session(&mut ws, long_lived_cmd()).await;

    ws.send(control(&ClientMsg::Attach { id, channel: 1, last_seq: None, cols: 80, rows: 24 }))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    ws.send(input_frame(1, b"echo BEFORE_DROP\r")).await.unwrap();
    read_channel_until(&mut ws, 1, "BEFORE_DROP").await;

    // 记录断点:重连时从 0 开始请求——0 仍在 1MiB 缓冲内,应得到 replay
    drop(ws);
    let mut ws2 = connect(&url).await;
    ws2.send(control(&ClientMsg::Attach { id, channel: 5, last_seq: Some(0), cols: 80, rows: 24 }))
        .await
        .unwrap();
    match next_control(&mut ws2).await {
        ServerMsg::Attached { channel: 5, mode: AttachMode::Replay, .. } => {}
        other => panic!("unexpected: {other:?}"),
    }
    let replayed = read_channel_until(&mut ws2, 5, "BEFORE_DROP").await;
    assert!(replayed.contains("BEFORE_DROP"));
    ws2.send(control(&ClientMsg::KillSession { id })).await.unwrap();
}

#[tokio::test]
async fn stale_seq_gets_resync() {
    let url = start_test_server("t").await;
    // 用极小缓冲的服务端逼出超窗:start_test_server 固定 1MiB,这里直接以 last_seq 超过 end_seq 触发 resync 分支
    let mut ws = connect(&url).await;
    let id = create_session(&mut ws, long_lived_cmd()).await;
    ws.send(control(&ClientMsg::Attach { id, channel: 1, last_seq: Some(u64::MAX), cols: 80, rows: 24 }))
        .await
        .unwrap();
    match next_control(&mut ws).await {
        ServerMsg::Attached { channel: 1, mode: AttachMode::Resync, .. } => {}
        other => panic!("unexpected: {other:?}"),
    }
    assert!(matches!(next_control(&mut ws).await, ServerMsg::ResyncBegin { channel: 1 }));
    loop {
        if let ServerMsg::ResyncEnd { channel: 1, .. } = next_control(&mut ws).await {
            break;
        }
    }
    ws.send(control(&ClientMsg::KillSession { id })).await.unwrap();
}

#[tokio::test]
async fn duplicate_channel_rejected() {
    let url = start_test_server("t").await;
    let mut ws = connect(&url).await;
    let id = create_session(&mut ws, long_lived_cmd()).await;
    ws.send(control(&ClientMsg::Attach { id, channel: 1, last_seq: None, cols: 80, rows: 24 }))
        .await
        .unwrap();
    loop {
        if let ServerMsg::ResyncEnd { .. } = next_control(&mut ws).await { break; }
    }
    ws.send(control(&ClientMsg::Attach { id, channel: 1, last_seq: None, cols: 80, rows: 24 }))
        .await
        .unwrap();
    assert!(matches!(next_control(&mut ws).await, ServerMsg::Error { .. }));
    ws.send(control(&ClientMsg::KillSession { id })).await.unwrap();
}
