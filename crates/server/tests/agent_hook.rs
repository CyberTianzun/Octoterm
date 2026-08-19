//! hook 摄入面。
//!
//! 这是第三方入口,鉴权与客户端控制面是分开的两套 —— 客户端用 bearer token,
//! agent 用 hook 密钥,后者只存在于 octoterm 亲手 spawn 的会话的环境变量里。

mod common;

use octoterm_server::agent::store::hook_token;

async fn post_hook(
    addr: std::net::SocketAddr,
    path: &str,
    token: Option<&str>,
    session: Option<&str>,
    body: &str,
) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(format!("http://{addr}{path}"))
        .header("Content-Type", "application/json")
        .body(body.to_string());
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    if let Some(s) = session {
        req = req.header("X-Octoterm-Session", s);
    }
    req.send().await.unwrap()
}

async fn agent_sessions(addr: std::net::SocketAddr, token: &str) -> serde_json::Value {
    reqwest::Client::new()
        .get(format!("http://{addr}/api/agents/sessions"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .map(|t| serde_json::from_str(&t).unwrap())
        .unwrap()
}

async fn server() -> std::net::SocketAddr {
    common::start_test_server_at("tok", 1 << 16, Default::default(), vec![]).await
}

#[tokio::test]
async fn missing_auth_is_rejected() {
    let addr = server().await;
    let r = post_hook(addr, "/hook/claude-code/stop", None, Some("1"), "{}").await;
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn wrong_token_is_rejected() {
    let addr = server().await;
    let r = post_hook(addr, "/hook/claude-code/stop", Some("nope"), Some("1"), "{}").await;
    assert_eq!(r.status(), 401);
}

/// 客户端那个 bearer token **不能**用来打 hook 面:两套凭据,两个信任域。
#[tokio::test]
async fn client_token_does_not_work_on_hook_plane() {
    let addr = server().await;
    let r = post_hook(addr, "/hook/claude-code/stop", Some("tok"), Some("1"), "{}").await;
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn missing_session_header_is_rejected() {
    let addr = server().await;
    let r = post_hook(addr, "/hook/claude-code/stop", Some(hook_token()), None, "{}").await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn unknown_agent_is_404() {
    let addr = server().await;
    let r = post_hook(addr, "/hook/no-such-agent/stop", Some(hook_token()), Some("1"), "{}").await;
    assert_eq!(r.status(), 404);
}

/// agent 升级会带来新事件。收下、忽略、回 200 —— 绝不能因为多了一个事件名就把
/// Claude 卡住或者报错。
#[tokio::test]
async fn unknown_event_is_ignored_not_an_error() {
    let addr = server().await;
    let r = post_hook(
        addr,
        "/hook/claude-code/some-future-event",
        Some(hook_token()),
        Some("1"),
        r#"{"session_id":"s1"}"#,
    )
    .await;
    assert_eq!(r.status(), 200);
    let v = agent_sessions(addr, "tok").await;
    assert!(v["sessions"].as_array().unwrap().is_empty(), "未知事件不该创建会话");
}

#[tokio::test]
async fn telemetry_updates_state() {
    let addr = server().await;
    let r = post_hook(
        addr,
        "/hook/claude-code/pre-tool-use",
        Some(hook_token()),
        Some("7"),
        r#"{"session_id":"s1","tool_name":"Bash","cwd":"/tmp/x"}"#,
    )
    .await;
    assert_eq!(r.status(), 200);

    let v = agent_sessions(addr, "tok").await;
    let s = &v["sessions"][0];
    assert_eq!(s["agent_id"], "claude-code");
    assert_eq!(s["agent_session_id"], "s1");
    assert_eq!(s["session"], 7);
    assert_eq!(s["state"], "working");
    assert_eq!(s["detail"], "Bash");
    assert_eq!(s["cwd"], "/tmp/x");
}

/// 「在等人」是整条链路里唯一必须精确的状态,它只从 Notification 来。
#[tokio::test]
async fn notification_marks_waiting() {
    let addr = server().await;
    post_hook(
        addr,
        "/hook/claude-code/notification",
        Some(hook_token()),
        Some("7"),
        r#"{"session_id":"s1","notification_type":"permission_prompt"}"#,
    )
    .await;
    let v = agent_sessions(addr, "tok").await;
    assert_eq!(v["sessions"][0]["state"], "waiting");
    assert_eq!(v["sessions"][0]["detail"], "permission_prompt");
}

/// 不假设 `SessionStart` 一定先到 —— Task 3 的端到端里实测过 `-p` 模式下第一个
/// 到达的是 `UserPromptSubmit`。任何事件都要能惰性建出会话。
#[tokio::test]
async fn any_event_can_create_the_session() {
    let addr = server().await;
    post_hook(
        addr,
        "/hook/claude-code/user-prompt-submit",
        Some(hook_token()),
        Some("3"),
        r#"{"session_id":"late"}"#,
    )
    .await;
    let v = agent_sessions(addr, "tok").await;
    assert_eq!(v["sessions"][0]["agent_session_id"], "late");
    assert_eq!(v["sessions"][0]["state"], "thinking");
}

/// 只带 tool_name 的后续事件不该把先前记下的 cwd 抹掉。
#[tokio::test]
async fn partial_update_does_not_clear_known_fields() {
    let addr = server().await;
    post_hook(
        addr,
        "/hook/claude-code/session-start",
        Some(hook_token()),
        Some("5"),
        r#"{"session_id":"s9","cwd":"/work/repo"}"#,
    )
    .await;
    post_hook(
        addr,
        "/hook/claude-code/pre-tool-use",
        Some(hook_token()),
        Some("5"),
        r#"{"session_id":"s9","tool_name":"Edit"}"#,
    )
    .await;
    let v = agent_sessions(addr, "tok").await;
    assert_eq!(v["sessions"][0]["cwd"], "/work/repo", "cwd 被后续事件抹掉了");
    assert_eq!(v["sessions"][0]["state"], "working");
}
