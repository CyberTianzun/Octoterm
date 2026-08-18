use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use octoterm_desktop::supervisor::Supervisor;
use octoterm_protocol::{ClientMsg, Frame, ServerMsg, CONTROL_CHANNEL, PROTO_VERSION};
use octoterm_server::config::WindowSize;
use tokio_tungstenite::tungstenite::Message;

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

/// 收口:`Supervisor::restart` 是所有 token 进入 `AppState` 的必经之路,空 token
/// 一律拒绝 —— server 侧 `bearer_ok` 与 WebSocket 握手都是 `token == state.token`
/// 的直接比较,空对空就是把鉴权整个关掉。
///
/// 关键在于「**不产生任何副作用**」:同地址那条路径是先 `stop()` 再 bind,拦得晚
/// 一步就会变成「已经把旧的关了,才发现 token 是空的」—— 用户原来还能用的服务被
/// 一次失败的保存弄没了。所以这里既断言返回 Err,也断言旧的 listener 仍在跑。
#[tokio::test]
async fn an_empty_token_is_refused_without_touching_the_running_listener() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[]);
    let addr = sup.restart("127.0.0.1:0".parse().unwrap(), "t".into()).await.unwrap();

    // 同地址 + 空 token:最危险的那一组(先 stop 再 bind)
    let err = sup.restart(addr, String::new()).await.unwrap_err();
    assert!(format!("{err:#}").contains("空 token"), "{err:#}");
    // 纯空白同样不算 token
    let err = sup.restart(addr, "   ".into()).await.unwrap_err();
    assert!(format!("{err:#}").contains("空 token"), "{err:#}");

    assert_eq!(sup.listen(), Some(addr), "拒绝之后不该动已有的 HTTP 层");
    assert_eq!(sup.token(), Some("t"), "旧 token 应当原封不动");
    assert!(tokio::net::TcpStream::connect(addr).await.is_ok(), "旧 listener 被误关了");
}

/// 从来没起来过的时候也一样拒绝,而不是「反正没有东西可丢,就让它跑起来」。
#[tokio::test]
async fn an_empty_token_is_refused_even_when_nothing_is_running() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[]);
    let err = sup.restart("127.0.0.1:0".parse().unwrap(), String::new()).await.unwrap_err();

    assert!(format!("{err:#}").contains("空 token"), "{err:#}");
    assert_eq!(sup.listen(), None, "拒绝之后不该有 HTTP 层");
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

// ---- 下面这条测试专门复现「持有活跃 WebSocket 时同地址重启」----
//
// 注意:这条测试在 macOS 上恒绿**不代表功能没问题**,真正的判据在 Windows。
//
// 链条是这样的:`restart` 走「地址没变、只改 token」时是先 `stop()` 再带重试地
// bind;而 `stop()` 的 abort 够不着已经升级完成的 WebSocket —— axum 的 ws 回调跑在
// `on_upgrade` 自己 `tokio::spawn` 出去的独立任务里。那条连接的 TCP socket,其
// local port 就是监听端口,而且按 octoterm 的产品设计它永远不主动关闭。于是重新
// bind 时,同一个 local port 上还挂着一批 ESTABLISHED socket。
//
// mio 只在 Unix 给 TcpListener 设 `SO_REUSEADDR`,Windows 上**明确不设**(见 mio
// 的 `src/net/tcp/listener.rs`,注释里指向 MS 文档)。所以 macOS 有 `SO_REUSEADDR`
// 兜底,问题被完全遮住;Windows 上则可能 bind 不回来 —— 而这条路径是先 stop 后
// bind 的,重试全败就会停在「完全没有 HTTP 层」的状态上。
//
// 这条测试的价值就在于挂上 Windows CI 之后能把它暴露出来。

type Ws = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

fn control(msg: &ClientMsg) -> Message {
    Message::Binary(Frame::new(CONTROL_CHANNEL, serde_json::to_vec(msg).unwrap()).encode().into())
}

fn parse_control(msg: Message) -> Option<ServerMsg> {
    let Message::Binary(data) = msg else { return None };
    let frame = Frame::decode(&data).unwrap();
    (frame.channel == CONTROL_CHANNEL).then(|| serde_json::from_slice(&frame.payload).unwrap())
}

/// 连上并**完成握手**(必须走到 upgrade 完成:普通 HTTP keep-alive 连接在 abort 时
/// 会被 graceful_shutdown 收掉,只有已升级的 WS 才会存活下来)。
async fn connect_authed(addr: std::net::SocketAddr, token: &str) -> Ws {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws")).await.unwrap();
    ws.send(control(&ClientMsg::Hello { token: token.into(), proto: PROTO_VERSION }))
        .await
        .unwrap();
    loop {
        if let Some(ServerMsg::HelloOk { .. }) = parse_control(ws.next().await.unwrap().unwrap()) {
            break;
        }
    }
    ws
}

#[tokio::test]
async fn restarting_the_same_address_while_a_websocket_is_still_connected() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[]);
    let addr = sup.restart("127.0.0.1:0".parse().unwrap(), "old".into()).await.unwrap();

    // 握手完成的活连接,整条测试期间**不断开**(underscore 绑定只是压警告,
    // 生命周期到函数结束才结束)。
    let _held = connect_authed(addr, "old").await;

    // 同地址、只换 token:先 stop 后 bind,此时旧连接还占着这个 local port。
    let again = sup.restart(addr, "new".into()).await.unwrap();

    assert_eq!(addr, again, "同地址重启不该换地址");
    assert_eq!(sup.token(), Some("new"), "token 应当已经轮换");

    // 新地址必须真的能用:不光 TCP 连得上,还要能走完新 token 的握手。
    let _fresh = connect_authed(again, "new").await;
}

/// 端到端钉死这个 crate 存在的理由:**restart 之后的新 HTTP 层,看到的必须是同一个
/// `SessionManager`**。
///
/// 为什么不能只断言 `sup.manager().list().len()`:那读的是 Supervisor 自己那份
/// `Arc<SessionManager>`,restart 里就算把 `AppState.manager` 换成一个全新的
/// manager,这个断言照样绿 —— 而「新 HTTP 层接的是旧 manager」正是承诺本身。
/// 所以这里必须绕出去,从**新地址**用新 token 握手,拿服务端返回的会话列表说话。
#[tokio::test]
async fn a_session_created_before_a_rebind_is_visible_from_the_new_address() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[]);
    let old = sup.restart("127.0.0.1:0".parse().unwrap(), "t1".into()).await.unwrap();

    let session = sup.manager().create(Some("survivor".into()), long_lived_cmd(), None).unwrap();
    let id = session.id;

    // 换地址 + 换 token:两样都变,新 HTTP 层是彻底重建出来的
    let new = sup.restart("127.0.0.1:0".parse().unwrap(), "t2".into()).await.unwrap();
    assert_ne!(old, new, "端口 0 每次应当分到不同端口");

    let mut ws = connect_authed(new, "t2").await;
    ws.send(control(&ClientMsg::ListSessions)).await.unwrap();
    let sessions = loop {
        if let Some(ServerMsg::Sessions { sessions }) =
            parse_control(ws.next().await.unwrap().unwrap())
        {
            break sessions;
        }
    };

    let found = sessions.iter().find(|s| s.id == id).unwrap_or_else(|| {
        panic!("restart 之后新 HTTP 层看不到 rebind 前建的会话,manager 被重建了:{sessions:?}")
    });
    assert_eq!(found.name, "survivor");

    sup.manager().kill(id);
}
