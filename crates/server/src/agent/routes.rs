//! agent 集成的 HTTP 面。
//!
//! 走 `/api/` 而不是控制消息:这些请求**与会话无关、低频、且客户端在 socket 起来
//! 之前就要用**(设置页一打开就要看到列表)。更硬的理由是协议 X3 —— 新增
//! client→server 控制消息是破坏性变更,要 bump proto 并让所有已打开的页面全断;
//! 为一个可以用 HTTP 表达的低频请求付这个代价不值。

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::agent::detect::DetectEnv;
use crate::app::{bearer_ok, AppState};

/// `GET /api/agents` —— 本机装了哪些 agent、我方集成是什么状态。
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !bearer_ok(&headers, &state.token) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let port = state.listen_port;
    // 扫描要读若干配置文件和 PATH,是阻塞 IO,不能占着 async 执行器
    match tokio::task::spawn_blocking(move || crate::agent::scan(&DetectEnv::current(), port)).await
    {
        Ok(agents) => Json(serde_json::json!({ "agents": agents })).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "agent 扫描任务失败");
            (StatusCode::INTERNAL_SERVER_ERROR, "agent scan failed").into_response()
        }
    }
}
