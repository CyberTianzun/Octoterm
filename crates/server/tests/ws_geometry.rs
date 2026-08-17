//! 多端同时 attach 一个会话时的尺寸归并(G1–G8)。
//!
//! pty 只有一个尺寸,所有 attach 收到的是同一份字节流,所以 attach/resize 只是
//! 尺寸"诉求",服务端按 window-size 策略归并出权威值,再用 `resized` 告知每一端。

mod common;
use common::{
    connect, control, create_session, long_lived_cmd, next_control, start_test_server,
    start_test_server_with, Ws,
};
use futures_util::SinkExt;
use octoterm_protocol::{ClientMsg, ServerMsg};
use octoterm_server::config::WindowSize;
use std::time::Duration;

/// 等这个 channel 上的下一条 `resized`,忽略途中的其它控制消息。
async fn next_resized(ws: &mut Ws, channel: u32) -> (u16, u16) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline - tokio::time::Instant::now();
        match tokio::time::timeout(left, next_control(ws)).await {
            Ok(ServerMsg::Resized { channel: ch, cols, rows }) if ch == channel => {
                return (cols, rows)
            }
            Ok(_) => continue,
            Err(_) => panic!("no resized on channel {channel} within 10s"),
        }
    }
}

async fn attach(ws: &mut Ws, id: u64, channel: u32, cols: u16, rows: u16) {
    ws.send(control(&ClientMsg::Attach { id, channel, last_seq: None, cols, rows })).await.unwrap();
}

/// 后到的小窗口把 pty 压到两个维度各自的最小值,并且**两端**都被告知新尺寸——
/// 先到的那一端如果不知道,就会按 100 列去渲染 80 列的字节流,画面直接烂掉。
#[tokio::test]
async fn smallest_wins_and_both_ends_are_told() {
    let url = start_test_server("t").await;
    let mut a = connect(&url).await;
    let id = create_session(&mut a, long_lived_cmd()).await;

    attach(&mut a, id, 1, 100, 30).await;
    assert_eq!(next_resized(&mut a, 1).await, (100, 30), "独此一家时就用它请求的尺寸");

    // B 更窄但更高:两个维度分别取最小
    let mut b = connect(&url).await;
    attach(&mut b, id, 2, 80, 40).await;
    assert_eq!(next_resized(&mut b, 2).await, (80, 30), "B 不该拿到自己请求的 80×40");
    assert_eq!(next_resized(&mut a, 1).await, (80, 30), "A 必须被告知尺寸已经变小");

    a.send(control(&ClientMsg::KillSession { id })).await.unwrap();
}

/// detach 之后那一端的诉求必须立刻从表里摘掉,否则一个已经走掉的小窗口会把
/// 剩下的人永远锁在小尺寸上。
#[tokio::test]
async fn detach_releases_the_constraint() {
    let url = start_test_server("t").await;
    let mut a = connect(&url).await;
    let id = create_session(&mut a, long_lived_cmd()).await;

    attach(&mut a, id, 1, 120, 40).await;
    assert_eq!(next_resized(&mut a, 1).await, (120, 40));
    let mut b = connect(&url).await;
    attach(&mut b, id, 2, 60, 20).await;
    assert_eq!(next_resized(&mut a, 1).await, (60, 20));

    b.send(control(&ClientMsg::Detach { channel: 2 })).await.unwrap();
    assert_eq!(next_resized(&mut a, 1).await, (120, 40), "detach 后应恢复到 A 的尺寸");

    a.send(control(&ClientMsg::KillSession { id })).await.unwrap();
}

/// 连接直接断开(手机息屏、网络掉线)和显式 detach 必须等价:Attachment 析构
/// 时席位跟着摘除。
#[tokio::test]
async fn dropped_connection_releases_the_constraint() {
    let url = start_test_server("t").await;
    let mut a = connect(&url).await;
    let id = create_session(&mut a, long_lived_cmd()).await;

    attach(&mut a, id, 1, 120, 40).await;
    assert_eq!(next_resized(&mut a, 1).await, (120, 40));
    let mut b = connect(&url).await;
    attach(&mut b, id, 2, 60, 20).await;
    assert_eq!(next_resized(&mut a, 1).await, (60, 20));

    drop(b);
    assert_eq!(next_resized(&mut a, 1).await, (120, 40), "掉线后应恢复到 A 的尺寸");

    a.send(control(&ClientMsg::KillSession { id })).await.unwrap();
}

/// 端到端:归并结果必须真的落到 pty 上(TIOCSWINSZ),而不只是一条消息——
/// 会话里的程序看到的 `stty size` 就是权威尺寸。
#[cfg(unix)]
#[tokio::test]
async fn merged_size_reaches_the_pty() {
    use common::{input_frame, read_channel_until};

    let url = start_test_server("t").await;
    let mut a = connect(&url).await;
    let id = create_session(&mut a, long_lived_cmd()).await;

    attach(&mut a, id, 1, 100, 30).await;
    assert_eq!(next_resized(&mut a, 1).await, (100, 30));
    let mut b = connect(&url).await;
    attach(&mut b, id, 2, 80, 24).await;
    assert_eq!(next_resized(&mut a, 1).await, (80, 24));

    tokio::time::sleep(Duration::from_millis(300)).await; // 等 shell 就绪
    a.send(input_frame(1, b"stty size\r")).await.unwrap();
    let got = read_channel_until(&mut a, 1, "24 80").await;
    assert!(got.contains("24 80"), "pty 没有拿到归并后的尺寸,输出: {got}");

    a.send(control(&ClientMsg::KillSession { id })).await.unwrap();
}

/// 极端上报(软键盘把视口顶没了)被夹到下界,而不是把会话压死、连累其他人。
#[tokio::test]
async fn extreme_request_is_clamped() {
    let url = start_test_server("t").await;
    let mut a = connect(&url).await;
    let id = create_session(&mut a, long_lived_cmd()).await;

    attach(&mut a, id, 1, 1, 1).await;
    assert_eq!(next_resized(&mut a, 1).await, (20, 5));

    a.send(control(&ClientMsg::KillSession { id })).await.unwrap();
}

/// latest 策略下由最近一次 attach/resize 说了算,和 smallest 是两条不同的路。
#[tokio::test]
async fn latest_policy_follows_the_most_recent_request() {
    let url = start_test_server_with("t", 1 << 20, WindowSize::Latest).await;
    let mut a = connect(&url).await;
    let id = create_session(&mut a, long_lived_cmd()).await;

    attach(&mut a, id, 1, 100, 30).await;
    assert_eq!(next_resized(&mut a, 1).await, (100, 30));

    let mut b = connect(&url).await;
    attach(&mut b, id, 2, 120, 40).await;
    assert_eq!(next_resized(&mut b, 2).await, (120, 40), "B 是最近的一次,它说了算");
    assert_eq!(next_resized(&mut a, 1).await, (120, 40), "smallest 下这里会是 100×30");

    // A 再动一次窗口,权威又回到 A
    a.send(control(&ClientMsg::Resize { channel: 1, cols: 90, rows: 25 })).await.unwrap();
    assert_eq!(next_resized(&mut b, 2).await, (90, 25));

    a.send(control(&ClientMsg::KillSession { id })).await.unwrap();
}
