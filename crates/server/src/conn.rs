use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use base64::Engine;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use octoterm_protocol::{AttachMode, ClientMsg, Frame, ServerMsg, CONTROL_CHANNEL};
use tokio::sync::mpsc;

use crate::app::AppState;
use crate::session::pty::{Session, SessionOutput, Viewport};

const COALESCE_MAX: usize = 64 * 1024;

/// 依据水位线裁剪广播消息:去掉已经通过 replay/resync 送达客户端的前缀。
/// 返回需要发送的字节(None = 整条已覆盖);推进水位线到 end_seq。
fn trim_by_watermark(watermark: &mut u64, end_seq: u64, bytes: &[u8]) -> Option<Vec<u8>> {
    let start = end_seq.saturating_sub(bytes.len() as u64);
    if end_seq <= *watermark {
        return None; // 完全在已送达范围内,水位线不动
    }
    let skip = watermark.saturating_sub(start) as usize;
    *watermark = end_seq;
    Some(bytes[skip..].to_vec())
}

pub(crate) fn control_frame(msg: &ServerMsg) -> Message {
    Message::Binary(Frame::new(CONTROL_CHANNEL, serde_json::to_vec(msg).unwrap()).encode().into())
}

/// 告知这个 channel 会话的权威尺寸。返回 false 表示连接已断。
async fn send_resized(out: &mpsc::Sender<Message>, channel: u32, (cols, rows): (u16, u16)) -> bool {
    out.send(control_frame(&ServerMsg::Resized { channel, cols, rows })).await.is_ok()
}

/// 发送一次完整的 resync:快照 → Resized → ResyncBegin → 重绘帧 → ResyncEnd{seq},
/// 并把水位线推进到快照的 end_seq。任意一步发送失败则返回 false(调用方应结束泵)。
async fn send_resync(
    session: &Arc<Session>,
    channel: u32,
    out: &mpsc::Sender<Message>,
    watermark: &mut u64,
) -> bool {
    let snap = session.snapshot();
    // 重绘是整屏的,客户端必须先按重绘所用的尺寸调整,再吃这一帧(G6)
    if !send_resized(out, channel, (snap.cols, snap.rows)).await {
        return false;
    }
    if out.send(control_frame(&ServerMsg::ResyncBegin { channel })).await.is_err() {
        return false;
    }
    if out
        .send(Message::Binary(Frame::new(channel, snap.repaint).encode().into()))
        .await
        .is_err()
    {
        return false;
    }
    if out
        .send(control_frame(&ServerMsg::ResyncEnd { channel, seq: snap.end_seq }))
        .await
        .is_err()
    {
        return false;
    }
    *watermark = snap.end_seq;
    true
}

struct Attachment {
    session: Arc<Session>,
    /// 本 attach 在会话尺寸表里的席位:Attachment 一旦从 map 里移除(detach、
    /// 或连接结束时整张表被 drop),它跟着析构,会话尺寸自动重算。
    viewport: Viewport,
    pump: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
struct ConnState {
    attachments: HashMap<u32, Attachment>,
}

pub async fn run(socket: WebSocket, state: AppState) {
    let (sink, stream) = socket.split();
    // 所有出站消息统一走 out 队列(容量 64,写端在慢客户端上自然阻塞)
    let (out, out_rx) = mpsc::channel::<Message>(64);
    let writer = tokio::spawn(write_loop(sink, out_rx));

    // 会话事件推送
    let mut events = state.manager.events();
    let out_events = out.clone();
    let event_task = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(msg) => {
                    if out_events.send(control_frame(&msg)).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    let mut conn = ConnState::default();
    read_loop(stream, &state, &out, &mut conn).await;

    for (_, a) in conn.attachments {
        a.pump.abort();
    }
    event_task.abort();
    drop(out);
    let _ = writer.await;
}

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(90);

/// 每 30s 主动发一个 Ping,让中间的代理/负载均衡不至于因为连接"看起来空闲"
/// 而把它掐断;同时给 read_loop 的 90s 超时判活打个底(客户端的 Pong 会重置
/// 那个超时——见 read_loop 的注释)。
async fn write_loop(mut sink: SplitSink<WebSocket, Message>, mut rx: mpsc::Receiver<Message>) {
    let mut keepalive =
        tokio::time::interval_at(tokio::time::Instant::now() + KEEPALIVE_INTERVAL, KEEPALIVE_INTERVAL);
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Some(msg) => {
                        if sink.send(msg).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = keepalive.tick() => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = sink.close().await;
}

async fn read_loop(
    mut stream: SplitStream<WebSocket>,
    state: &AppState,
    out: &mpsc::Sender<Message>,
    conn: &mut ConnState,
) {
    loop {
        // 90s 内一帧都没收到(包括 Pong)就认为连接已经死了:pumps 由 run()
        // 里既有的清理逻辑负责收尾,这里只需要退出、让上层做 abort/drop。
        let next = match tokio::time::timeout(READ_TIMEOUT, stream.next()).await {
            Ok(next) => next,
            Err(_) => break,
        };
        let Some(Ok(msg)) = next else { break };
        let Message::Binary(data) = msg else { continue };
        let Ok(frame) = Frame::decode(&data) else { continue };
        if frame.channel == CONTROL_CHANNEL {
            match serde_json::from_slice::<ClientMsg>(&frame.payload) {
                Ok(msg) => handle_control(msg, state, out, conn).await,
                Err(e) => {
                    tracing::warn!(error = %e, "malformed control message");
                    send_err(out, "malformed control message").await;
                }
            }
        } else {
            match conn.attachments.get(&frame.channel) {
                Some(a) => {
                    if let Err(e) = a.session.write_input(&frame.payload) {
                        tracing::warn!(channel = frame.channel, error = %e, "session input failed");
                        send_err_ch(out, &format!("session input failed: {e:#}"), frame.channel)
                            .await;
                    }
                }
                None => send_err_ch(out, "no such channel", frame.channel).await,
            }
        }
    }
}

async fn send_err(out: &mpsc::Sender<Message>, message: &str) {
    let _ = out.send(control_frame(&ServerMsg::Error { message: message.into(), channel: None })).await;
}

/// 带 channel 上下文的错误:attach/detach/resize/input 失败时用这个,
/// 客户端才能把错误关联到具体的 channel(比如据此关闭对应的终端页面)。
async fn send_err_ch(out: &mpsc::Sender<Message>, message: &str, channel: u32) {
    let _ = out
        .send(control_frame(&ServerMsg::Error { message: message.into(), channel: Some(channel) }))
        .await;
}

async fn handle_control(
    msg: ClientMsg,
    state: &AppState,
    out: &mpsc::Sender<Message>,
    conn: &mut ConnState,
) {
    match msg {
        ClientMsg::Hello { .. } => send_err(out, "already authenticated").await,
        ClientMsg::ListSessions => {
            let _ = out
                .send(control_frame(&ServerMsg::Sessions { sessions: state.manager.list() }))
                .await;
        }
        ClientMsg::NewSession { name, command, cwd } => {
            match state.manager.create(name, command, cwd) {
                Ok(s) => tracing::info!(session = s.id, "new-session ok"),
                Err(e) => {
                    tracing::error!(error = %e, "new-session failed");
                    send_err(out, &format!("spawn failed: {e:#}")).await;
                }
            }
            // 成功时 Created 事件经事件推送到达客户端
        }
        ClientMsg::KillSession { id } => {
            if !state.manager.kill(id) {
                tracing::warn!(session = id, "kill-session: no such session");
                send_err(out, "no such session").await;
            }
        }
        ClientMsg::RenameSession { id, name } => {
            if !state.manager.rename(id, &name) {
                send_err(out, "no such session").await;
            }
        }
        ClientMsg::Preview { id } => match state.manager.get(id) {
            Some(session) => {
                let snap = session.snapshot();
                let data = base64::engine::general_purpose::STANDARD.encode(snap.repaint);
                let _ = out.send(control_frame(&ServerMsg::PreviewData { id, data })).await;
            }
            None => send_err(out, "no such session").await,
        },
        ClientMsg::Attach { id, channel, last_seq, cols, rows } => {
            if channel == CONTROL_CHANNEL || conn.attachments.contains_key(&channel) {
                return send_err_ch(out, "channel unavailable", channel).await;
            }
            let Some(session) = state.manager.get(id) else {
                tracing::warn!(session = id, channel, "attach: no such session");
                return send_err_ch(out, "no such session", channel).await;
            };
            // 先登记尺寸诉求再订阅:这次 attach 引起的尺寸变化广播给已有的
            // attach 就够了,本端的权威尺寸由下面的 resized 一并带出去。
            let viewport = session.viewport(cols, rows);
            let rx = session.subscribe(); // 先订阅再快照,不丢中间字节

            // 决定 replay 还是 resync,并发送恢复数据
            let replay = last_seq.and_then(|seq| session.replay_from(seq));
            let (mode, seq) = match &replay {
                Some((end_seq, _)) => (AttachMode::Replay, *end_seq),
                None => (AttachMode::Resync, 0), // resync 的权威 seq 在 ResyncEnd 里
            };
            let _ = out.send(control_frame(&ServerMsg::Attached { channel, seq, mode })).await;
            let mut watermark = 0;
            match replay {
                Some((end_seq, bytes)) => {
                    // 权威尺寸未必是本端请求的那个(取决于 window-size 策略),
                    // 画面字节之前先告诉客户端该按多大渲染(G6)
                    send_resized(out, channel, session.size()).await;
                    if !bytes.is_empty() {
                        let _ = out
                            .send(Message::Binary(Frame::new(channel, bytes).encode().into()))
                            .await;
                    }
                    watermark = end_seq;
                }
                // resync 自带 resized(见 send_resync)
                None => {
                    send_resync(&session, channel, out, &mut watermark).await;
                }
            }

            let pump =
                tokio::spawn(pump_output(channel, id, session.clone(), rx, out.clone(), watermark));
            conn.attachments.insert(channel, Attachment { session, viewport, pump });
        }
        ClientMsg::Detach { channel } => match conn.attachments.remove(&channel) {
            Some(a) => a.pump.abort(), // a 在此析构 → viewport 摘除 → 尺寸重算
            None => send_err_ch(out, "no such channel", channel).await,
        },
        ClientMsg::Resize { channel, cols, rows } => match conn.attachments.get(&channel) {
            Some(a) => {
                if let Err(e) = a.viewport.set(cols, rows) {
                    tracing::warn!(channel, error = %e, "resize failed");
                    send_err_ch(out, &format!("resize failed: {e:#}"), channel).await;
                }
            }
            None => send_err_ch(out, "no such channel", channel).await,
        },
    }
}

async fn pump_output(
    channel: u32,
    session_id: u64,
    session: Arc<Session>,
    mut rx: tokio::sync::broadcast::Receiver<SessionOutput>,
    out: mpsc::Sender<Message>,
    mut watermark: u64,
) {
    use tokio::sync::broadcast::error::{RecvError, TryRecvError};
    loop {
        match rx.recv().await {
            Ok(SessionOutput::Data { end_seq, bytes }) => {
                // 合帧:非阻塞追加积压数据,单帧不超过 64 KiB;每条消息先过水位线去重
                let mut buf = trim_by_watermark(&mut watermark, end_seq, &bytes).unwrap_or_default();
                let mut exited = false;
                // tokio broadcast 只报告一次 Lagged:下一轮 try_recv()/recv() 会
                // 直接跳到间隙之后的数据,若不在这里显式 resync 就会静默丢字节。
                let mut lagged = false;
                let mut resized = None;
                loop {
                    match rx.try_recv() {
                        Ok(SessionOutput::Data { end_seq, bytes }) => {
                            let Some(trimmed) = trim_by_watermark(&mut watermark, end_seq, &bytes)
                            else {
                                continue;
                            };
                            if !buf.is_empty()
                                && buf.len() + trimmed.len() > COALESCE_MAX
                                && out
                                    .send(Message::Binary(
                                        Frame::new(channel, std::mem::take(&mut buf)).encode().into(),
                                    ))
                                    .await
                                    .is_err()
                            {
                                return;
                            }
                            buf.extend_from_slice(&trimmed);
                        }
                        // 尺寸变化必须夹在正确的位置:它之前的字节按旧尺寸渲染,
                        // 之后的按新尺寸。先把已合的帧发出去,再发 resized。
                        Ok(SessionOutput::Resized { cols, rows }) => {
                            resized = Some((cols, rows));
                            break;
                        }
                        Ok(SessionOutput::Exited) => {
                            exited = true;
                            break;
                        }
                        Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
                        Err(TryRecvError::Lagged(_)) => {
                            lagged = true;
                            break;
                        }
                    }
                }
                if !buf.is_empty()
                    && out
                        .send(Message::Binary(Frame::new(channel, buf).encode().into()))
                        .await
                        .is_err()
                {
                    return;
                }
                if let Some(size) = resized
                    && !send_resized(&out, channel, size).await
                {
                    return;
                }
                if exited {
                    let _ = out
                        .send(control_frame(&ServerMsg::SessionExited { channel, id: session_id }))
                        .await;
                    return;
                }
                if lagged {
                    // 排空剩余积压(非阻塞,与下方 outer Lagged 分支同语义):
                    // 途中遇到 Exited 直接通知退出并结束泵,不再走 resync。
                    loop {
                        match rx.try_recv() {
                            Ok(SessionOutput::Exited) => {
                                let _ = out
                                    .send(control_frame(&ServerMsg::SessionExited {
                                        channel,
                                        id: session_id,
                                    }))
                                    .await;
                                return;
                            }
                            // 丢弃的字节和途中的尺寸变化都由紧接着的 resync 兜底:
                            // 它带着当前的权威尺寸和整屏重绘(G6)。
                            Ok(SessionOutput::Data { .. })
                            | Ok(SessionOutput::Resized { .. }) => continue,
                            Err(TryRecvError::Lagged(_)) => continue,
                            Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
                        }
                    }
                    if !send_resync(&session, channel, &out, &mut watermark).await {
                        return;
                    }
                }
            }
            Ok(SessionOutput::Resized { cols, rows }) => {
                if !send_resized(&out, channel, (cols, rows)).await {
                    return;
                }
            }
            Ok(SessionOutput::Exited) => {
                let _ = out
                    .send(control_frame(&ServerMsg::SessionExited { channel, id: session_id }))
                    .await;
                return;
            }
            Err(RecvError::Lagged(_)) => {
                // 慢客户端:丢弃积压,resync 到最新画面。若排空过程中遇到 Exited,
                // 直接通知退出并结束泵,不再走 resync。
                loop {
                    match rx.try_recv() {
                        Ok(SessionOutput::Exited) => {
                            let _ = out
                                .send(control_frame(&ServerMsg::SessionExited {
                                    channel,
                                    id: session_id,
                                }))
                                .await;
                            return;
                        }
                        // 丢弃的字节和途中的尺寸变化都由紧接着的 resync 兜底:
                        // 它带着当前的权威尺寸和整屏重绘(G6)。
                        Ok(SessionOutput::Data { .. } | SessionOutput::Resized { .. }) => continue,
                        Err(TryRecvError::Lagged(_)) => continue,
                        Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
                    }
                }
                if !send_resync(&session, channel, &out, &mut watermark).await {
                    return;
                }
            }
            Err(RecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::trim_by_watermark;

    #[test]
    fn fully_covered_message_returns_none_and_leaves_watermark() {
        let mut watermark = 100u64;
        let bytes = b"hello".to_vec(); // covers [95, 100)
        let out = trim_by_watermark(&mut watermark, 100, &bytes);
        assert_eq!(out, None);
        assert_eq!(watermark, 100);
    }

    #[test]
    fn partially_covered_message_trims_prefix_and_advances_watermark() {
        let mut watermark = 3u64;
        let bytes = b"abcde".to_vec(); // covers [0, 5); watermark at 3 means [0,3) already delivered
        let out = trim_by_watermark(&mut watermark, 5, &bytes);
        assert_eq!(out, Some(b"de".to_vec()));
        assert_eq!(watermark, 5);
    }

    #[test]
    fn fully_new_message_passes_through_whole_and_advances_watermark() {
        let mut watermark = 0u64;
        let bytes = b"xyz".to_vec(); // covers [0, 3), nothing delivered yet
        let out = trim_by_watermark(&mut watermark, 3, &bytes);
        assert_eq!(out, Some(b"xyz".to_vec()));
        assert_eq!(watermark, 3);
    }
}
