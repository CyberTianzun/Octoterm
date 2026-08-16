use octoterm_server::session::pty::{Session, SessionOutput};
use std::time::Duration;

// 延迟输出,保证测试先完成 subscribe(真实消费者 attach 时同样先订阅再快照)
fn echo_cmd(text: &str) -> Option<Vec<String>> {
    #[cfg(unix)]
    return Some(vec![
        "/bin/sh".into(),
        "-c".into(),
        format!("sleep 0.3; printf '{text}'"),
    ]);
    #[cfg(windows)]
    return Some(vec![
        "powershell.exe".into(),
        "-Command".into(),
        format!("Start-Sleep -Milliseconds 300; Write-Host {text}"),
    ]);
}

async fn collect_until_exit(session: &Session) -> Vec<u8> {
    let mut rx = session.subscribe();
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Ok(SessionOutput::Data { bytes, .. })) => out.extend_from_slice(&bytes),
            Ok(Ok(SessionOutput::Exited)) => break,
            Ok(Err(_)) | Err(_) => break,
        }
    }
    out
}

#[tokio::test]
async fn spawned_command_output_is_broadcast_and_recorded() {
    let s = Session::spawn(1, "t".into(), 80, 24, echo_cmd("MARKER_A"), 1 << 20).unwrap();
    let out = collect_until_exit(&s).await;
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("MARKER_A"), "got: {text}");

    // 环形缓冲同步记录了同样的字节
    let (end_seq, replay) = s.replay_from(0).unwrap();
    assert_eq!(end_seq as usize, replay.len());
    assert!(String::from_utf8_lossy(&replay).contains("MARKER_A"));

    // grid 也消费了输出
    let snap = s.snapshot();
    assert_eq!(snap.end_seq, end_seq);
    assert!(String::from_utf8_lossy(&snap.repaint).contains("MARKER_A"));
}

#[tokio::test]
async fn write_input_reaches_child() {
    // 交互式 shell 回显输入
    let s = Session::spawn(2, "t".into(), 80, 24, None, 1 << 20).unwrap();
    let mut rx = s.subscribe();
    tokio::time::sleep(Duration::from_millis(300)).await; // 等 shell 就绪
    s.write_input(b"echo MARKER_B\r").unwrap();
    let mut acc = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Ok(SessionOutput::Data { bytes, .. })) => {
                acc.extend_from_slice(&bytes);
                if String::from_utf8_lossy(&acc).matches("MARKER_B").count() >= 2 {
                    break; // 回显 + 命令输出
                }
            }
            _ => {}
        }
    }
    assert!(String::from_utf8_lossy(&acc).contains("MARKER_B"));
    s.kill();
}

#[tokio::test]
async fn kill_terminates_session() {
    let s = Session::spawn(3, "t".into(), 80, 24, None, 1 << 20).unwrap();
    let mut rx = s.subscribe();
    tokio::time::sleep(Duration::from_millis(300)).await;
    s.kill();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(tokio::time::Instant::now() < deadline, "no Exited after kill");
        match tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            Ok(Ok(SessionOutput::Exited)) => break,
            Ok(Err(_)) => break,
            _ => {}
        }
    }
}
