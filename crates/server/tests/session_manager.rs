use octoterm_server::config::WindowSize;
use octoterm_protocol::{ServerMsg, SessionEventKind};
use octoterm_server::session::manager::SessionManager;
use std::time::Duration;

#[tokio::test]
async fn fast_exiting_command_is_removed() {
    let m = SessionManager::new(1 << 20, WindowSize::default());
    let mut events = m.events();
    #[cfg(unix)]
    let cmd = Some(vec!["/bin/sh".into(), "-c".into(), "exit 0".into()]);
    #[cfg(windows)]
    let cmd = Some(vec!["cmd.exe".into(), "/C".into(), "exit 0".into()]);
    let _s = m.create(None, cmd, None).unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        assert!(tokio::time::Instant::now() < deadline, "session was never removed");
        match tokio::time::timeout(std::time::Duration::from_secs(1), events.recv()).await {
            Ok(Ok(ServerMsg::SessionEvent { event: SessionEventKind::Closed, .. })) => break,
            _ => continue,
        }
    }
    assert!(m.list().is_empty());
}

#[tokio::test]
async fn create_list_rename_kill() {
    let m = SessionManager::new(1 << 20, WindowSize::default());
    let mut events = m.events();

    let s = m.create(Some("work".into()), None, None).unwrap();
    assert_eq!(m.list().len(), 1);
    assert_eq!(m.list()[0].name, "work");
    match events.recv().await.unwrap() {
        ServerMsg::SessionEvent { event: SessionEventKind::Created, session } => {
            assert_eq!(session.id, s.id)
        }
        other => panic!("unexpected: {other:?}"),
    }

    assert!(m.rename(s.id, "renamed"));
    assert_eq!(m.list()[0].name, "renamed");
    match events.recv().await.unwrap() {
        ServerMsg::SessionEvent { event: SessionEventKind::Renamed, .. } => {}
        other => panic!("unexpected: {other:?}"),
    }

    assert!(m.kill(s.id));
    // kill 必须立刻从列表摘掉,不能等 ConPTY 读线程收尸(Windows 上可能永远等不到)
    assert!(m.list().is_empty());
    // 同时仍广播 Closed,客户端靠这个刷新 UI
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(tokio::time::Instant::now() < deadline);
        match tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
            Ok(Ok(ServerMsg::SessionEvent { event: SessionEventKind::Closed, .. })) => break,
            _ => continue,
        }
    }
    assert!(m.list().is_empty());
    assert!(!m.rename(999, "x"));
    assert!(!m.kill(999));
}

#[tokio::test]
async fn default_name_uses_id() {
    let m = SessionManager::new(1 << 20, WindowSize::default());
    let s = m.create(None, None, None).unwrap();
    assert_eq!(m.list()[0].name, format!("octoterm-{}", s.id));
    m.kill(s.id);
}
