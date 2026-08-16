mod common;
use common::{control, parse_server, start_test_server, start_test_server_with_cap};
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

/// 快速产生大量输出、然后保持存活(便于测试结束时 kill),用来把 256 槽的
/// broadcast 挤爆,逼出服务端的 Lagged 处理路径。
fn bulk_producer_cmd() -> Vec<String> {
    #[cfg(unix)]
    return vec![
        "/bin/sh".into(),
        "-c".into(),
        "yes xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx | head -c 10000000; sleep 30".into(),
    ];
    #[cfg(windows)]
    return vec![
        "powershell.exe".into(),
        "-Command".into(),
        "$s='x'*8192; for($i=0;$i -lt 1500;$i++){Write-Host $s}; Start-Sleep 30".into(),
    ];
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

/// cap=64 的环形缓冲很快就会把 seq 0 挤出窗口:重连时请求 last_seq=0
/// 应该拿到 Resync(而不是 replay),随后走 ResyncBegin → 重绘帧 → ResyncEnd。
#[tokio::test]
async fn evicted_seq_downgrades_to_resync() {
    let url = start_test_server_with_cap("t", 64).await;
    let mut ws = connect(&url).await;
    let id = create_session(&mut ws, long_lived_cmd()).await;

    ws.send(control(&ClientMsg::Attach { id, channel: 1, last_seq: None, cols: 80, rows: 24 }))
        .await
        .unwrap();
    match next_control(&mut ws).await {
        ServerMsg::Attached { channel: 1, mode: AttachMode::Resync, .. } => {}
        other => panic!("unexpected: {other:?}"),
    }
    loop {
        if let ServerMsg::ResyncEnd { channel: 1, .. } = next_control(&mut ws).await {
            break;
        }
    }

    // 反复输出,确保产生的字节数远超 64 字节的缓冲容量,把 seq 0 挤出窗口。
    for _ in 0..5 {
        ws.send(input_frame(1, b"echo AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\r")).await.unwrap();
    }
    let mut total = 0usize;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while total <= 64 && tokio::time::Instant::now() < deadline {
        let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_millis(500), ws.next()).await
        else {
            continue;
        };
        if let Some((1, Err(bytes))) = parse_server(msg) {
            total += bytes.len();
        }
    }
    assert!(total > 64, "did not observe enough output to evict seq 0, got {total} bytes");

    drop(ws);
    let mut ws2 = connect(&url).await;
    ws2.send(control(&ClientMsg::Attach { id, channel: 2, last_seq: Some(0), cols: 80, rows: 24 }))
        .await
        .unwrap();
    match next_control(&mut ws2).await {
        ServerMsg::Attached { channel: 2, mode: AttachMode::Resync, .. } => {}
        other => panic!("unexpected: {other:?}"),
    }
    assert!(matches!(next_control(&mut ws2).await, ServerMsg::ResyncBegin { channel: 2 }));
    loop {
        if let ServerMsg::ResyncEnd { channel: 2, .. } = next_control(&mut ws2).await {
            break;
        }
    }
    ws2.send(control(&ClientMsg::KillSession { id })).await.unwrap();
}

/// 一个不读 socket 的慢客户端应该让服务端观测到 broadcast 的 Lagged,并主动
/// 发出 ResyncBegin/ResyncEnd,而不是静默丢字节。
///
/// 用带插桩的临时构建验证过:在这台 macOS 开发机上,这条 pty 的单次 read
/// 稳定被截断在 ~1KB,且吞吐被逐条节流(约 1.6ms/条),导致 pump_output 里
/// 合帧用的内层 `try_recv` 循环几乎不可能一次性攒够 COALESCE_MAX(64KiB)再
/// 遇到 Lagged——实测里挤压是通过 outer `RecvError::Lagged` 分支(修复前就
/// 已经存在且正确)被吸收掉的。也就是说,在这台机器上单靠这个黑盒测试并不能
/// 把 pre-fix/post-fix 两版代码的行为区分开(两版都能通过)。测试仍然保留,
/// 因为它验证了"lag 后必须 resync、不能静默丢字节"这一更宽的用户可见契约,
/// 并且确实会执行到 commit 2 重构出的 `send_resync` 辅助函数;但它不能作为
/// commit 2 内层分支修复的独立回归证据——详见 fix report 里 commit 3 的
/// RED 证据小节。
#[tokio::test]
async fn slow_reader_gets_lag_resync() {
    let url = start_test_server("t").await;
    let mut ws = connect(&url).await;
    let id = create_session(&mut ws, Some(bulk_producer_cmd())).await;

    // last_seq: Some(0) → 会话刚创建、buffer 里 seq 0 显然还在,attach 走 replay
    // 分支(不会自带 ResyncBegin/ResyncEnd),这样后面观察到的 resync 只可能来自
    // lag 触发,不会跟 attach 自带的初始 resync 混淆。
    ws.send(control(&ClientMsg::Attach { id, channel: 1, last_seq: Some(0), cols: 80, rows: 24 }))
        .await
        .unwrap();

    // 完全不读 socket:server 端 out mpsc(cap 64)与 session 的 broadcast
    // (cap 256)会被 bulk_producer_cmd 挤爆,逼出 Lagged。
    tokio::time::sleep(Duration::from_secs(3)).await;

    let mut saw_begin = false;
    let mut saw_end = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline && !(saw_begin && saw_end) {
        let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(1), ws.next()).await
        else {
            continue;
        };
        if let Some((0, Ok(m))) = parse_server(msg) {
            match m {
                ServerMsg::ResyncBegin { channel: 1 } => saw_begin = true,
                ServerMsg::ResyncEnd { channel: 1, .. } if saw_begin => saw_end = true,
                _ => {}
            }
        }
    }
    assert!(saw_begin, "never observed ResyncBegin on channel 1 after inducing lag");
    assert!(saw_end, "never observed ResyncEnd on channel 1 after inducing lag");

    ws.send(control(&ClientMsg::KillSession { id })).await.unwrap();
}
