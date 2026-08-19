use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use futures_util::SinkExt;
use octoterm_protocol::{ClientMsg, Frame, ServerMsg, CONTROL_CHANNEL, PROTO_VERSION};

use crate::launcher::LauncherProvider;
use crate::session::manager::SessionManager;
use tokio::time::timeout;

#[derive(Clone)]
pub struct AppState {
    pub manager: Arc<SessionManager>,
    pub token: String,
    /// 新建会话菜单的来源。进程启动时装配一次,每次请求重新扫描(见 launcher 模块)。
    pub launchers: Arc<Vec<Box<dyn LauncherProvider>>>,
    /// 实际监听的端口。装 hook 时要把它写进 URL,判定「装了却端口对不上」也要它。
    /// 取的是 listener 的 `local_addr()` 而不是配置值 —— 配 `:0` 时两者不同。
    pub listen_port: u16,
    /// `[agents]` 配置。装 hook 的门控就在这里,默认关。
    pub agents: crate::config::AgentsConfig,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/ws", any(ws_handler))
        .route("/api/launchers", get(launchers_handler))
        .route("/api/agents", get(crate::agent::routes::list))
        .route("/api/agents/{id}/plan", get(crate::agent::routes::plan))
        .route("/api/agents/{id}/install", post(crate::agent::routes::install))
        .route("/api/agents/{id}/uninstall", post(crate::agent::routes::uninstall))
        .fallback(crate::assets::static_handler)
        .with_state(state)
}

/// `GET /api/launchers` —— 新建会话时可选的启动项。
///
/// 走 HTTP 而不是控制消息:这是一份**与会话无关的静态清单**,客户端在页面加载时
/// 就要用(那时 WebSocket 可能还没握手完),而且它天然适合被当成一次性请求 ——
/// 塞进控制通道只会给协议增加一对无状态的请求/响应,并且要为它发明一个关联键
/// (协议里没有 request id,见 docs/protocol.md C5)。
async fn launchers_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !bearer_ok(&headers, &state.token) {
        // 和 WebSocket 握手同一个 token。用 Authorization 头而不是查询参数:
        // 查询参数会进日志/历史记录,而且带头部的请求必须由 JS 发出,顺手挡掉了
        // 从别的页面用 <form>/<img> 打过来的跨站请求。
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    // 扫描要读若干配置文件,是阻塞 IO,不能占着 async 执行器
    let providers = state.launchers.clone();
    match tokio::task::spawn_blocking(move || crate::launcher::discover_all(&providers)).await {
        Ok(list) => Json(serde_json::json!({ "launchers": list })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "launcher 扫描任务失败");
            (StatusCode::INTERNAL_SERVER_ERROR, "launcher discovery failed").into_response()
        }
    }
}

pub(crate) fn bearer_ok(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|v| v == token)
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
    let first = timeout(Duration::from_secs(5), socket.recv()).await;
    let data = match first {
        Err(_) => return reject(socket, "hello timeout").await,
        Ok(None) | Ok(Some(Err(_))) => return false, // 连接已断,无处可回
        Ok(Some(Ok(Message::Binary(data)))) => data,
        Ok(Some(Ok(_))) => return reject(socket, "expected binary hello frame").await,
    };
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
        _ => reject(socket, "bad hello").await,
    }
}

async fn reject(socket: &mut WebSocket, message: &str) -> bool {
    tracing::warn!(reason = message, "websocket handshake rejected");
    let _ = socket.send(control_msg(&ServerMsg::Error { message: message.into(), channel: None })).await;
    false
}

async fn handle_socket(mut socket: WebSocket, state: AppState) -> anyhow::Result<()> {
    if !handshake(&mut socket, &state).await {
        let _ = socket.close().await;
        return Ok(());
    }
    crate::conn::run(socket, state).await;
    Ok(())
}
