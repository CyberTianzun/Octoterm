use axum::extract::ws::{Message, WebSocket};
use base64::Engine;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use octoterm_protocol::{ClientMsg, Frame, ServerMsg, CONTROL_CHANNEL};
use tokio::sync::mpsc;

use crate::app::AppState;

pub(crate) fn control_frame(msg: &ServerMsg) -> Message {
    Message::Binary(Frame::new(CONTROL_CHANNEL, serde_json::to_vec(msg).unwrap()).encode().into())
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

    read_loop(stream, &state, &out).await;

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

async fn read_loop(mut stream: SplitStream<WebSocket>, state: &AppState, out: &mpsc::Sender<Message>) {
    while let Some(Ok(msg)) = stream.next().await {
        let Message::Binary(data) = msg else { continue };
        let Ok(frame) = Frame::decode(&data) else { continue };
        if frame.channel == CONTROL_CHANNEL {
            match serde_json::from_slice::<ClientMsg>(&frame.payload) {
                Ok(msg) => handle_control(msg, state, out).await,
                Err(_) => send_err(out, "malformed control message").await,
            }
        } else {
            // Task 9: 输入帧路由到已 attach 的会话
            send_err(out, "attach not implemented").await;
        }
    }
}

async fn send_err(out: &mpsc::Sender<Message>, message: &str) {
    let _ = out.send(control_frame(&ServerMsg::Error { message: message.into() })).await;
}

async fn handle_control(msg: ClientMsg, state: &AppState, out: &mpsc::Sender<Message>) {
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
        ClientMsg::Attach { .. } | ClientMsg::Detach { .. } | ClientMsg::Resize { .. } => {
            send_err(out, "attach not implemented").await;
        }
    }
}
