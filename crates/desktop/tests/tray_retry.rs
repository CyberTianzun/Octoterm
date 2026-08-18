//! 托盘创建失败后的重试节奏。
//!
//! 这条逻辑值得钉死,是因为它兜的那个坑代价极高:托盘是这个进程唯一的界面,建
//! 不出来又不重试、不退出,进程就变成「用户既看不见也关不掉」的僵尸——HTTP 层
//! 占着端口,单实例锁挡着重开,只能去任务管理器杀。
//!
//! 「弹框」「退出」这两个动作本身没法在单测里验证(要真的起事件循环、还要有人去
//! 点那个模态框),但**什么时候该走到那一步**是纯粹的算术,在这里测。

use std::time::Duration;

use octoterm_desktop::app::tray_retry_delay;

#[test]
fn the_first_failure_is_retried_soon() {
    // Windows 开机自启撞上 explorer.exe 没就绪,通常几百毫秒就好了 —— 第一次
    // 重试要快,不能让用户对着空菜单栏干等。
    assert_eq!(tray_retry_delay(1), Some(Duration::from_millis(500)));
}

#[test]
fn the_delay_backs_off() {
    let delays: Vec<_> = (1..=4).map(|n| tray_retry_delay(n).unwrap()).collect();
    assert!(
        delays.windows(2).all(|w| w[1] > w[0]),
        "退避必须是递增的,否则失败时会高频空转:{delays:?}"
    );
}

#[test]
fn retrying_gives_up_eventually() {
    // 关键的一条:必须有尽头。永远重试 = 永远不弹框、永远不退出 = 僵尸进程。
    assert_eq!(tray_retry_delay(5), None);
    assert_eq!(tray_retry_delay(6), None);
    assert_eq!(
        tray_retry_delay(u32::MAX),
        None,
        "不能溢出,也不能又开始重试"
    );
}

#[test]
fn the_total_wait_covers_the_login_window_without_stalling_the_user() {
    let total: Duration = (1..).map_while(tray_retry_delay).sum();
    assert!(
        (Duration::from_secs(5)..=Duration::from_secs(15)).contains(&total),
        "累计等待要够覆盖登录时 explorer 起来的这段,又不能让用户干等太久:{total:?}"
    );
}
