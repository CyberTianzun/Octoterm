use std::time::Duration;

use octoterm_desktop::supervisor::Supervisor;
use octoterm_server::config::WindowSize;

/// 一个不会自己退出的会话,测完由 kill 收尾。
fn long_lived_cmd() -> Option<Vec<String>> {
    #[cfg(unix)]
    return Some(vec!["/bin/sh".into(), "-i".into()]);
    #[cfg(windows)]
    return None; // 默认 powershell
}

/// abort 之后端口的释放是异步的,给它一点时间。
async fn wait_until_refused(addr: std::net::SocketAddr) -> bool {
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_err() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn rebinding_to_a_new_port_keeps_sessions_alive() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[]);
    let old = sup.restart("127.0.0.1:0".parse().unwrap(), "t1".into()).await.unwrap();

    sup.manager().create(None, long_lived_cmd(), None).unwrap();
    assert_eq!(sup.manager().list().len(), 1);

    let new = sup.restart("127.0.0.1:0".parse().unwrap(), "t1".into()).await.unwrap();
    assert_ne!(old, new, "端口 0 每次应当分到不同端口");

    assert!(wait_until_refused(old).await, "旧端口仍在接受连接");
    assert!(tokio::net::TcpStream::connect(new).await.is_ok(), "新端口不可连");
    assert_eq!(sup.manager().list().len(), 1, "rebind 不该丢会话");

    let id = sup.manager().list()[0].id;
    sup.manager().kill(id);
}

#[tokio::test]
async fn changing_only_the_token_reuses_the_same_address() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[]);
    let addr = sup.restart("127.0.0.1:0".parse().unwrap(), "old".into()).await.unwrap();

    // 同地址重启:先关后 bind + 重试,必须仍然成功且地址不变
    let again = sup.restart(addr, "new".into()).await.unwrap();
    assert_eq!(addr, again);
    assert_eq!(sup.token(), Some("new"));
    assert!(tokio::net::TcpStream::connect(again).await.is_ok());
}

#[tokio::test]
async fn a_failed_bind_leaves_the_old_listener_running() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[]);
    let addr = sup.restart("127.0.0.1:0".parse().unwrap(), "t".into()).await.unwrap();

    // 占住另一个端口,再让 supervisor 去抢它
    let squatter = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let taken = squatter.local_addr().unwrap();

    assert!(sup.restart(taken, "t".into()).await.is_err(), "抢占用端口应当失败");
    assert_eq!(sup.listen(), Some(addr), "失败后应当仍在原地址上");
    assert!(tokio::net::TcpStream::connect(addr).await.is_ok(), "旧 listener 被误关了");
}

#[tokio::test]
async fn stop_releases_the_port_but_not_the_sessions() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[]);
    let addr = sup.restart("127.0.0.1:0".parse().unwrap(), "t".into()).await.unwrap();
    sup.manager().create(None, long_lived_cmd(), None).unwrap();

    sup.stop();

    assert_eq!(sup.listen(), None);
    assert!(wait_until_refused(addr).await);
    assert_eq!(sup.manager().list().len(), 1, "停 HTTP 层不该动会话");

    let id = sup.manager().list()[0].id;
    sup.manager().kill(id);
}
