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

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path};

use crate::agent::apply::{apply, default_backup_dir, ApplyError, ApplyOpts};
use crate::agent::detect::DetectEnv;
use crate::agent::edit::{ConfigEdit, EditOp, InstallCtx};
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

/// 一条编辑的对外投影。不直接序列化 `EditOp` —— 那是内部结构,客户端不该依赖它
/// (R13:客户端中立)。
fn describe(e: &ConfigEdit) -> serde_json::Value {
    let (action, event, spec) = match &e.op {
        EditOp::EnsureHook { event, spec } => ("ensure", event, Some(spec.clone())),
        EditOp::RemoveOurs { event } => ("remove", event, None),
    };
    serde_json::json!({
        "path": e.path.to_string_lossy(),
        "action": action,
        "event": event,
        "spec": spec,
    })
}

fn ctx_for(state: &AppState, id: &str, include_blocking: bool) -> Option<(Box<dyn crate::agent::AgentAdapter>, InstallCtx)> {
    let adapter = crate::agent::find(id)?;
    let env = DetectEnv::current();
    Some((adapter, InstallCtx { home: env.home, port: state.listen_port, include_blocking }))
}

/// 装决策类 hook 会覆盖别家的决策(实测:最后注册的赢),所以默认只在**没有冲突**
/// 时才带上它;有冲突时要客户端显式 `?blocking=1`,并且它应当先把 conflicts 给
/// 用户看过。
fn wants_blocking(state: &AppState, id: &str, explicit: Option<bool>) -> bool {
    if let Some(v) = explicit {
        return v;
    }
    let env = DetectEnv::current();
    let ctx = InstallCtx { home: env.home, port: state.listen_port, include_blocking: true };
    crate::agent::find(id).map(|a| a.integration(&ctx).1.is_empty()).unwrap_or(false)
}

/// `GET /api/agents/{id}/plan` —— 预演。只读,返回将要做的编辑。
///
/// 装 hook 改的是**用户的**文件,所以必须能先看后装。
pub async fn plan(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !bearer_ok(&headers, &state.token) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let blocking = wants_blocking(&state, &id, None);
    let Some((adapter, ctx)) = ctx_for(&state, &id, blocking) else {
        return (StatusCode::NOT_FOUND, "no such agent").into_response();
    };
    match (adapter.plan_install(&ctx), adapter.plan_uninstall(&ctx)) {
        (Ok(install), Ok(uninstall)) => Json(serde_json::json!({
            "install": install.iter().map(describe).collect::<Vec<_>>(),
            "uninstall": uninstall.iter().map(describe).collect::<Vec<_>>(),
            "include_blocking": blocking,
            "install_enabled": state.agents.install_enabled,
        }))
        .into_response(),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "plan failed").into_response(),
    }
}

pub async fn install(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    mutate(state, id, headers, true).await
}

pub async fn uninstall(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    mutate(state, id, headers, false).await
}

async fn mutate(state: AppState, id: String, headers: HeaderMap, installing: bool) -> Response {
    if !bearer_ok(&headers, &state.token) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    // 卸载永远覆盖全部事件,否则关掉开关再卸载会留下残留
    let blocking = if installing { wants_blocking(&state, &id, None) } else { true };
    let Some((adapter, ctx)) = ctx_for(&state, &id, blocking) else {
        return (StatusCode::NOT_FOUND, "no such agent").into_response();
    };
    let Some(backup_dir) = default_backup_dir() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "cannot resolve backup dir").into_response();
    };
    let opts = ApplyOpts {
        enabled: state.agents.install_enabled,
        backup_dir,
        backup_keep: 5,
    };
    let plan = if installing { adapter.plan_install(&ctx) } else { adapter.plan_uninstall(&ctx) };
    let Ok(plan) = plan else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "plan failed").into_response();
    };
    // 落盘是阻塞 IO
    match tokio::task::spawn_blocking(move || apply(&plan, &opts)).await {
        Ok(Ok(outcomes)) => Json(serde_json::json!({
            "changed": outcomes.iter().any(|o| o.changed),
            "include_blocking": blocking,
            "files": outcomes.iter().map(|o| serde_json::json!({
                "path": o.path.to_string_lossy(),
                "changed": o.changed,
                "backup": o.backup.as_ref().map(|p| p.to_string_lossy()),
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Ok(Err(e)) => {
            let code = match e {
                // 开关关着是「你没开这个功能」,不是服务器出错
                ApplyError::Disabled => StatusCode::FORBIDDEN,
                ApplyError::Invalid { .. } => StatusCode::CONFLICT,
                ApplyError::Io { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            };
            tracing::warn!(error = %e, "agent 集成写入失败");
            (code, e.to_string()).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "agent 集成写入任务失败");
            (StatusCode::INTERNAL_SERVER_ERROR, "apply task failed").into_response()
        }
    }
}

/// `GET /api/agents/sessions` —— agent 会话全量快照。
///
/// 客户端页面加载时、以及**每次重连后**都拉一次这个,不做增量对账(R6)。增量只由
/// `AgentEvent` 广播承担,断线期间漏掉的事件靠这次全量拉取补齐。
pub async fn sessions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !bearer_ok(&headers, &state.token) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    Json(serde_json::json!({ "sessions": state.agent_sessions.list() })).into_response()
}

/// `POST /hook/{agent}/{event}` —— agent 打进来的地方。
///
/// 这是**第三方入口**,不属于客户端控制面,因此:
/// - 用的是独立的 hook 密钥,不是客户端那个 bearer token;
/// - 只认回环地址,主监听是不是 0.0.0.0 都一样;
/// - 认不出的事件返回 200 而不是 4xx —— agent 升级会带来新事件,不能因此把它卡住。
pub async fn hook(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((agent, event_slug)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // 1. 只认回环。hook payload 里有 tool_input(命令原文、文件路径),不对外开。
    if !peer.ip().is_loopback() {
        tracing::warn!(%peer, "非回环地址访问 hook 面,拒绝");
        return StatusCode::FORBIDDEN.into_response();
    }
    // 2. hook 密钥。拿不到 = 不是我们托管的会话里跑的 agent(见 store::hook_token)。
    let ok = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|v| v == crate::agent::store::hook_token());
    if !ok {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // 3. 关联到哪个托管会话。**不校验会话是否还活着** —— 会话刚没、hook 还在路上是
    //    正常的时序,交给清理去收(Task 7),不必在这里制造一个失败。
    let session = headers
        .get("X-Octoterm-Session")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let Some(session) = session else {
        return (StatusCode::BAD_REQUEST, "missing or bad X-Octoterm-Session").into_response();
    };

    let Some(adapter) = crate::agent::find(&agent) else {
        return (StatusCode::NOT_FOUND, "no such agent").into_response();
    };

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) if body.is_empty() => serde_json::json!({}),
        Err(e) => {
            tracing::debug!(error = %e, "hook payload 不是合法 JSON");
            return (StatusCode::BAD_REQUEST, "bad json").into_response();
        }
    };

    let event = crate::agent::edit::event_of_slug(&event_slug);
    let Some(update) = adapter.parse(&event, &payload) else {
        // 认不出的事件:收下,忽略,回 200。不认识不是错误。
        tracing::debug!(agent = %agent, event = %event, "忽略未知 hook 事件");
        return StatusCode::OK.into_response();
    };

    let agent_session_id = payload
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("default")
        .to_string();

    let snapshot =
        state.agent_sessions.apply(adapter.id(), &agent_session_id, Some(session), update);
    state.manager.publish(snapshot.to_msg());
    StatusCode::OK.into_response()
}
