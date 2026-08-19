//! 「退出会杀掉全部会话」——本 task 最实质的一条逻辑,而且它**靠 drop 是做不到的**。
//!
//! `Session` 没有 `Drop` 实现,pty 的读线程和 wait 线程各自握着一份
//! `Arc<Session>`,`Arc` 计数永远归不了零,所以 `Session::kill()` 不会被自动调用。
//! `app::shutdown` 因此必须显式遍历 `manager.list()` 逐个 kill —— 这个测试就是
//! 用来钉死那个循环的:把它删掉,下面的断言会立刻红。

use std::time::Duration;

use octoterm_desktop::app::shutdown;
use octoterm_desktop::supervisor::Supervisor;
use octoterm_server::config::WindowSize;

/// 一个不会自己退出的会话,只能被 kill 收掉。
fn long_lived_cmd() -> Option<Vec<String>> {
    #[cfg(unix)]
    return Some(vec!["/bin/sh".into(), "-i".into()]);
    #[cfg(windows)]
    return None; // 默认 powershell
}

#[tokio::test]
async fn shutdown_kills_every_session() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[], Default::default());
    sup.restart("127.0.0.1:0".parse().unwrap(), "t".into()).await.unwrap();

    let a = sup.manager().create(None, long_lived_cmd(), None).unwrap();
    let b = sup.manager().create(None, long_lived_cmd(), None).unwrap();
    assert_eq!(sup.manager().list().len(), 2, "两个会话都该在列表里");
    assert!(!a.has_exited() && !b.has_exited(), "会话开局不该已经退出");

    shutdown(&mut sup);

    assert!(sup.manager().list().is_empty(), "退出后 manager 里不该还剩会话");
    assert!(sup.listen().is_none(), "退出后 HTTP 层该停了");

    // 从列表里摘掉只是记账,下面这段轮询证明的是「kill 确实走到了
    // force_close_pty,收尾线程也确实跑完了」—— 但不严格等于「子进程被 reap
    // 了」。`has_exited` 由读线程和 wait 线程任一个先跑到就会置位:`kill()`
    // 保证会关 pty master,读线程因此必然 EOF 并置位,这一步和子进程有没有真的
    // 退出无关;真正确认子进程收尸的是 wait 线程,而 `crates/server/src/session
    // /pty.rs` 自己写明 `WinChildKiller` 的返回值不可信。所以这条断言证明的是
    // 「kill 路径走到底、没有半途卡住」,不是「子进程一定被回收了」。
    for (name, s) in [("a", &a), ("b", &b)] {
        let mut exited = false;
        for _ in 0..200 {
            if s.has_exited() {
                exited = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(exited, "会话 {name} 的 kill 路径没有走到底(pty 一直没被拆掉)");
    }
}

/// 没有会话时 shutdown 也得干净收尾,不能 panic。
#[tokio::test]
async fn shutdown_without_sessions_is_a_noop() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[], Default::default());
    sup.restart("127.0.0.1:0".parse().unwrap(), "t".into()).await.unwrap();
    shutdown(&mut sup);
    assert!(sup.listen().is_none());
    assert!(sup.manager().list().is_empty());
}
