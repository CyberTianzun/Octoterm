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
use crate::agent::store::{AgentSessionStore, AnswerResult, Decision, PendingRequest, Update};
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
        EditOp::EnsureHook { event, group } => ("ensure", event, Some(group.clone())),
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

/// 认不出的调用一律 **200 + 空体**,而不是 4xx。
///
/// 这是实测教训。原先返回 401/400/404,结果 Claude Code 每轮都往用户脸上打一行
/// `Stop hook error: HTTP 401 from http://127.0.0.1:7683/hook/claude-code/stop` ——
/// 因为 agent 把 hook 的非 2xx 当成错误显示。
///
/// 而「不是托管会话打来的」是**完全正常**的事,不是错误:hook 装在用户级全局配置里,
/// 这台机器上每一个 Claude 会话都会触发它,其中绝大多数不是从 octoterm 里起的。
/// 把设计上必然发生的事报成错误,等于让用户天天看红字。
///
/// `200` + 空体对 agent 的语义恰好是「收到了,没有决定」,它照常走自己的流程。诊断
/// 信息也没丢:服务端按节流打日志,`GET /api/agents` 的自检也照样能报出装了却连不上。
fn ignored() -> Response {
    (StatusCode::OK, Json(serde_json::json!({}))).into_response()
}

/// 节流日志。每个工具调用都会来一次,不节流就是自己刷自己的日志。
fn log_ignored(reason: &str) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    const EVERY_SECS: u64 = 60;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let last = LAST.load(Ordering::Relaxed);
    if now.saturating_sub(last) >= EVERY_SECS
        && LAST.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_ok()
    {
        tracing::debug!(reason, "忽略了一个认不出的 hook 调用(60 秒内只记一条)");
    }
}

/// `POST /hook/{agent}/{event}` —— agent 打进来的地方。
///
/// 这是**第三方入口**,不属于客户端控制面,因此:
/// - 用的是独立的 hook 密钥,不是客户端那个 bearer token;
/// - 只认回环地址,主监听是不是 0.0.0.0 都一样;
/// - **认不出的一律安静地收下**(见 `ignored`),包括没鉴权、没会话头、不认识的 agent、
///   坏 JSON。安全性由「什么都不做」保证,而不是由回一个错误码保证。
pub async fn hook(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((agent, event_slug)): Path<(String, String)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // 只认回环。hook payload 里有 tool_input(命令原文、文件路径),不对外开。
    // 这一条保留 403:非回环的东西不是 agent,不需要对它安静。
    if !peer.ip().is_loopback() {
        tracing::warn!(%peer, "非回环地址访问 hook 面,拒绝");
        return StatusCode::FORBIDDEN.into_response();
    }
    // hook 密钥。拿不到 = 不是我们托管的会话里跑的 agent —— 正常情况,安静收下。
    let ok = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|v| v == crate::agent::store::hook_token());
    if !ok {
        log_ignored("no-or-bad-token");
        return ignored();
    }
    // 关联到哪个托管会话。**不校验会话是否还活着** —— 会话刚没、hook 还在路上是
    // 正常的时序,交给清理去收(见 store::decide),不必在这里制造一个失败。
    let Some(session) = headers
        .get("X-Octoterm-Session")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    else {
        log_ignored("no-session-header");
        return ignored();
    };

    let Some(adapter) = crate::agent::find(&agent) else {
        log_ignored("unknown-agent");
        return ignored();
    };

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) if body.is_empty() => serde_json::json!({}),
        Err(_) => {
            log_ignored("bad-json");
            return ignored();
        }
    };

    let event = crate::agent::edit::event_of_slug(&event_slug);
    let agent_session_id = payload
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("default")
        .to_string();

    if adapter.is_blocking(&event) {
        return blocking_hook(&state, adapter.as_ref(), &agent_session_id, session, &payload).await;
    }

    let Some(update) = adapter.parse(&event, &payload) else {
        // 认不出的事件:收下,忽略,回 200。不认识不是错误。
        tracing::debug!(agent = %agent, event = %event, "忽略未知 hook 事件");
        return ignored();
    };

    let snapshot =
        state.agent_sessions.apply(adapter.id(), &agent_session_id, Some(session), update);
    state.manager.publish(snapshot.to_msg());
    StatusCode::OK.into_response()
}

/// 挂起项的看守。
///
/// **这是本功能里最容易漏掉的一件事**:agent 侧断开连接(它超时了、崩了、用户
/// Ctrl-C 了)时,axum 会把这个 handler 的 future 直接丢掉 —— `await` 之后的代码
/// 一行都不会跑。只有 `Drop` 能保证挂起项被摘掉,否则它会一直挂到 590 秒,而客户端
/// 上会显示一个永远等不到答复的「有事找你」。
struct PendingGuard {
    store: std::sync::Arc<AgentSessionStore>,
    id: String,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.store.remove_pending(&self.id);
    }
}

async fn blocking_hook(
    state: &AppState,
    adapter: &dyn crate::agent::AgentAdapter,
    agent_session_id: &str,
    session: u64,
    payload: &serde_json::Value,
) -> Response {
    let tool_name = payload.get("tool_name").and_then(|v| v.as_str()).map(str::to_string);
    // 先确保会话存在,并把工具名带进去 —— 广播出去的 `AgentEvent.detail` 就是它。
    // 命令原文(`tool_input`)**不进广播**:控制通道有 4 KiB 上限且不许走大块数据
    // (协议 §10 / R4),客户端要详情就去拉 `/api/agents/pending`。
    state.agent_sessions.apply(
        adapter.id(),
        agent_session_id,
        Some(session),
        Update { detail: tool_name.clone(), ..Default::default() },
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let id = uuid::Uuid::new_v4().simple().to_string();
    let meta = PendingRequest {
        id: id.clone(),
        agent_id: adapter.id().to_string(),
        agent_session_id: agent_session_id.to_string(),
        session: Some(session),
        tool_name,
        tool_input: payload.get("tool_input").cloned().unwrap_or(serde_json::Value::Null),
        created_at: now,
        expires_at: now + state.agents.pending_timeout_secs,
    };

    let rx = state.agent_sessions.insert_pending(meta);
    let guard = PendingGuard { store: state.agent_sessions.clone(), id: id.clone() };
    if let Some(s) = state.agent_sessions.snapshot(adapter.id(), agent_session_id) {
        state.manager.publish(s.to_msg());
    }

    // 我们的超时必须短于写进 hook 的那个(600 秒):超时由我们主动写「无决定」,
    // 而不是让 Claude 那头自己超时 —— 行为一样,但这样我们知道发生了什么。
    let wait = std::time::Duration::from_secs(state.agents.pending_timeout_secs);
    let decision = match tokio::time::timeout(wait, rx).await {
        Ok(Ok(d)) => d,
        // 超时,或者答复端被丢弃 —— 一律「无决定」,绝不代替用户 allow/deny
        _ => {
            tracing::info!(pending = %id, "挂起请求未获答复,回落为无决定");
            Decision::NoDecision
        }
    };
    drop(guard);
    if let Some(s) = state.agent_sessions.snapshot(adapter.id(), agent_session_id) {
        state.manager.publish(s.to_msg());
    }
    Json(adapter.render(&decision)).into_response()
}

#[derive(serde::Deserialize)]
pub struct AnswerBody {
    pub pending_id: String,
    /// `"allow"` | `"deny"`。其他值一律 400 —— 不猜。
    pub decision: String,
    #[serde(default)]
    pub message: Option<String>,
}

/// `GET /api/agents/pending` —— 当前有哪些请求在等人。
pub async fn pending(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !bearer_ok(&headers, &state.token) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    Json(serde_json::json!({ "pending": state.agent_sessions.list_pending() })).into_response()
}

/// `POST /api/agents/answer` —— 替 agent 拍板。
///
/// 走 HTTP 而不是控制消息:新增 client→server 消息类型按 X3 是破坏性变更,要 bump
/// proto 并让所有已打开的页面全断。`pending_id` 就是协议 C5 说的「自然键」。
pub async fn answer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AnswerBody>,
) -> Response {
    if !bearer_ok(&headers, &state.token) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let decision = match body.decision.as_str() {
        "allow" => Decision::Allow { message: body.message },
        "deny" => Decision::Deny { message: body.message },
        other => {
            return (StatusCode::BAD_REQUEST, format!("unknown decision: {other}")).into_response()
        }
    };
    match state.agent_sessions.answer(&body.pending_id, decision) {
        AnswerResult::Ok => StatusCode::OK.into_response(),
        AnswerResult::NotFound => (StatusCode::NOT_FOUND, "no such pending request").into_response(),
        // 重复提交与「请求不存在」对客户端是两件事,不能混成同一个码
        AnswerResult::AlreadyAnswered => {
            (StatusCode::CONFLICT, "already answered").into_response()
        }
    }
}
