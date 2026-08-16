use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use base64::Engine;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use octoterm_protocol::{AttachMode, ClientMsg, Frame, ServerMsg, CONTROL_CHANNEL};
use tokio::sync::mpsc;

use crate::app::AppState;
use crate::session::pty::{Session, SessionOutput};

const COALESCE_MAX: usize = 64 * 1024;

pub(crate) fn control_frame(msg: &ServerMsg) -> Message {
    Message::Binary(Frame::new(CONTROL_CHANNEL, serde_json::to_vec(msg).unwrap()).encode().into())
}

struct Attachment {
    session: Arc<Session>,
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

async fn write_loop(mut sink: SplitSink<WebSocket, Message>, mut rx: mpsc::Receiver<Message>) {
    while let Some(msg) = rx.recv().await {
        if sink.send(msg).await.is_err() {
            break;
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
    while let Some(Ok(msg)) = stream.next().await {
        let Message::Binary(data) = msg else { continue };
        let Ok(frame) = Frame::decode(&data) else { continue };
        if frame.channel == CONTROL_CHANNEL {
            match serde_json::from_slice::<ClientMsg>(&frame.payload) {
                Ok(msg) => handle_control(msg, state, out, conn).await,
                Err(_) => send_err(out, "malformed control message").await,
            }
        } else {
            match conn.attachments.get(&frame.channel) {
                Some(a) => {
                    if a.session.write_input(&frame.payload).is_err() {
                        send_err(out, "session input failed").await;
                    }
                }
                None => send_err(out, "no such channel").await,
            }
        }
    }
}

async fn send_err(out: &mpsc::Sender<Message>, message: &str) {
    let _ = out.send(control_frame(&ServerMsg::Error { message: message.into() })).await;
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
        ClientMsg::NewSession { name, command } => {
            if let Err(e) = state.manager.create(name, command) {
                send_err(out, &format!("spawn failed: {e}")).await;
            }
            // 成功时 Created 事件经事件推送到达客户端
        }
        ClientMsg::KillSession { id } => {
            if !state.manager.kill(id) {
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
                return send_err(out, "channel unavailable").await;
            }
            let Some(session) = state.manager.get(id) else {
                return send_err(out, "no such session").await;
            };
            let rx = session.subscribe(); // 先订阅再快照,不丢中间字节
            let _ = session.resize(cols, rows);

            // 决定 replay 还是 resync,并发送恢复数据
            let replay = last_seq.and_then(|seq| session.replay_from(seq));
            let (mode, seq) = match &replay {
                Some((end_seq, _)) => (AttachMode::Replay, *end_seq),
                None => (AttachMode::Resync, 0), // resync 的权威 seq 在 ResyncEnd 里
            };
            let _ = out.send(control_frame(&ServerMsg::Attached { channel, seq, mode })).await;
            match replay {
                Some((_, bytes)) => {
                    if !bytes.is_empty() {
                        let _ = out
                            .send(Message::Binary(Frame::new(channel, bytes).encode().into()))
                            .await;
                    }
                }
                None => {
                    let snap = session.snapshot();
                    let _ = out.send(control_frame(&ServerMsg::ResyncBegin { channel })).await;
                    let _ = out
                        .send(Message::Binary(Frame::new(channel, snap.repaint).encode().into()))
                        .await;
                    let _ = out
                        .send(control_frame(&ServerMsg::ResyncEnd { channel, seq: snap.end_seq }))
                        .await;
                }
            }

            let pump = tokio::spawn(pump_output(channel, id, session.clone(), rx, out.clone()));
            conn.attachments.insert(channel, Attachment { session, pump });
        }
        ClientMsg::Detach { channel } => match conn.attachments.remove(&channel) {
            Some(a) => a.pump.abort(),
            None => send_err(out, "no such channel").await,
        },
        ClientMsg::Resize { channel, cols, rows } => match conn.attachments.get(&channel) {
            Some(a) => {
                let _ = a.session.resize(cols, rows);
            }
            None => send_err(out, "no such channel").await,
        },
    }
}

async fn pump_output(
    channel: u32,
    session_id: u64,
    session: Arc<Session>,
    mut rx: tokio::sync::broadcast::Receiver<SessionOutput>,
    out: mpsc::Sender<Message>,
) {
    use tokio::sync::broadcast::error::{RecvError, TryRecvError};
    loop {
        match rx.recv().await {
            Ok(SessionOutput::Data { bytes, .. }) => {
                // 合帧:非阻塞追加积压数据,上限 64 KiB
                let mut buf = bytes.to_vec();
                let mut exited = false;
                while buf.len() < COALESCE_MAX {
                    match rx.try_recv() {
                        Ok(SessionOutput::Data { bytes, .. }) => buf.extend_from_slice(&bytes),
                        Ok(SessionOutput::Exited) => {
                            exited = true;
                            break;
                        }
                        Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
                        Err(TryRecvError::Lagged(_)) => break, // 下轮 recv 处理
                    }
                }
                if out.send(Message::Binary(Frame::new(channel, buf).encode().into())).await.is_err() {
                    return;
                }
                if exited {
                    let _ = out
                        .send(control_frame(&ServerMsg::SessionExited { channel, id: session_id }))
                        .await;
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
                        Ok(SessionOutput::Data { .. }) => continue,
                        Err(TryRecvError::Lagged(_)) => continue,
                        Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
                    }
                }
                let snap = session.snapshot();
                let _ = out.send(control_frame(&ServerMsg::ResyncBegin { channel })).await;
                if out
                    .send(Message::Binary(Frame::new(channel, snap.repaint).encode().into()))
                    .await
                    .is_err()
                {
                    return;
                }
                let _ = out
                    .send(control_frame(&ServerMsg::ResyncEnd { channel, seq: snap.end_seq }))
                    .await;
            }
            Err(RecvError::Closed) => return,
        }
    }
}
