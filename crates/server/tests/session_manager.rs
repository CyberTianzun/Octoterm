use octoterm_protocol::{ServerMsg, SessionEventKind};
use octoterm_server::session::manager::SessionManager;
use std::time::Duration;

#[tokio::test]
async fn fast_exiting_command_is_removed() {
    let m = SessionManager::new(1 << 20);
    let mut events = m.events();
    #[cfg(unix)]
    let cmd = Some(vec!["/bin/sh".into(), "-c".into(), "exit 0".into()]);
    #[cfg(windows)]
    let cmd = Some(vec!["cmd.exe".into(), "/C".into(), "exit 0".into()]);
    let _s = m.create(None, cmd).unwrap();
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
    let m = SessionManager::new(1 << 20);
    let mut events = m.events();

    let s = m.create(Some("work".into()), None).unwrap();
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
    // 子进程退出后 manager 自动移除并广播 Closed
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
    let m = SessionManager::new(1 << 20);
    let s = m.create(None, None).unwrap();
    assert_eq!(m.list()[0].name, format!("octoterm-{}", s.id));
    m.kill(s.id);
}
