use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::routing::any;
use axum::Router;
use futures_util::SinkExt;
use octoterm_protocol::{ClientMsg, Frame, ServerMsg, CONTROL_CHANNEL, PROTO_VERSION};

use crate::session::manager::SessionManager;

#[derive(Clone)]
pub struct AppState {
    pub manager: Arc<SessionManager>,
    pub token: String,
}

pub fn router(state: AppState) -> Router {
    Router::new().route("/ws", any(ws_handler)).with_state(state)
}

pub async fn serve(listener: tokio::net::TcpListener, state: AppState) -> anyhow::Result<()> {
    axum::serve(listener, router(state)).await?;
    Ok(())
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| async move {
        let _ = handle_socket(socket, state).await;
    })
}

pub(crate) fn control_msg(msg: &ServerMsg) -> Message {
    Message::Binary(Frame::new(CONTROL_CHANNEL, serde_json::to_vec(msg).unwrap()).encode().into())
}

async fn handshake(socket: &mut WebSocket, state: &AppState) -> bool {
    let first = tokio::time::timeout(Duration::from_secs(5), socket.recv()).await;
    let Ok(Some(Ok(Message::Binary(data)))) = first else { return false };
    let hello = Frame::decode(&data)
        .ok()
        .filter(|f| f.channel == CONTROL_CHANNEL)
        .and_then(|f| serde_json::from_slice::<ClientMsg>(&f.payload).ok());
    match hello {
        Some(ClientMsg::Hello { token, proto })
            if token == state.token && proto == PROTO_VERSION =>
        {
            let _ = socket.send(control_msg(&ServerMsg::HelloOk { proto: PROTO_VERSION })).await;
            true
        }
        _ => {
            let _ = socket
                .send(control_msg(&ServerMsg::Error { message: "bad hello".into() }))
                .await;
            false
        }
    }
}

async fn handle_socket(mut socket: WebSocket, state: AppState) -> anyhow::Result<()> {
    if !handshake(&mut socket, &state).await {
        let _ = socket.close().await;
        return Ok(());
    }
    // Task 8 在此接管连接主循环;当前先拒绝一切后续消息
    while let Some(Ok(_)) = socket.recv().await {
        let _ = socket
            .send(control_msg(&ServerMsg::Error { message: "not implemented".into() }))
            .await;
    }
    Ok(())
}
