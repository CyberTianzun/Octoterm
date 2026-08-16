#![allow(dead_code)]

use octoterm_protocol::{ClientMsg, Frame, ServerMsg, CONTROL_CHANNEL};
use octoterm_server::app::{serve, AppState};
use octoterm_server::session::manager::SessionManager;
use tokio_tungstenite::tungstenite::Message;

pub async fn start_test_server(token: &str) -> String {
    start_test_server_with_cap(token, 1 << 20).await
}

pub async fn start_test_server_with_cap(token: &str, cap: usize) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = AppState { manager: SessionManager::new(cap), token: token.into() };
    tokio::spawn(async move { serve(listener, state).await.unwrap() });
    format!("ws://{addr}/ws")
}

pub fn control(msg: &ClientMsg) -> Message {
    Message::Binary(Frame::new(CONTROL_CHANNEL, serde_json::to_vec(msg).unwrap()).encode().into())
}

pub fn parse_server(msg: Message) -> Option<(u32, Result<ServerMsg, Vec<u8>>)> {
    let Message::Binary(data) = msg else { return None };
    let frame = Frame::decode(&data).unwrap();
    if frame.channel == CONTROL_CHANNEL {
        Some((frame.channel, Ok(serde_json::from_slice(&frame.payload).unwrap())))
    } else {
        Some((frame.channel, Err(frame.payload)))
    }
}
