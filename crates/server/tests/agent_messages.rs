//! `GET /api/agents/messages`:聊天视图的数据来源。
//!
//! 这一层的重点不是「读得到时返回什么」(那是 agent_window 的事),而是
//! **读不到时怎么说话** —— 抄 orca 的教训:回落必须带类型化的原因,绝不能返回一个
//! 空消息列表让客户端画出一个空聊天框。用户会以为对话真的是空的。

mod common;

use octoterm_server::agent::store::hook_token;
use octoterm_server::config::AgentsConfig;
use std::net::SocketAddr;

fn cfg(transcript: bool) -> AgentsConfig {
    AgentsConfig { transcript_enabled: transcript, ..Default::default() }
}

async fn server(transcript: bool) -> SocketAddr {
    common::start_test_server_with_agents("tok", 1 << 16, Default::default(), vec![], cfg(transcript))
        .await
}

/// 让服务端知道有这么一个 agent 会话,并(可选地)带上记录路径。
async fn seed(addr: SocketAddr, transcript: Option<&str>) {
    let body = match transcript {
        Some(p) => format!(r#"{{"session_id":"s1","transcript_path":"{p}"}}"#),
        None => r#"{"session_id":"s1"}"#.to_string(),
    };
    reqwest::Client::new()
        .post(format!("http://{addr}/hook/claude-code/session-start"))
        .header("Authorization", format!("Bearer {}", hook_token()))
        .header("X-Octoterm-Session", "1")
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
}

async fn messages(addr: SocketAddr, agent: &str, token: &str) -> (u16, serde_json::Value) {
    let r = reqwest::Client::new()
        .get(format!(
            "http://{addr}/api/agents/messages?agent_id={agent}&agent_session_id=s1"
        ))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    let text = r.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(serde_json::Value::Null))
}

#[tokio::test]
async fn requires_the_client_bearer() {
    let addr = server(true).await;
    let (status, _) = messages(addr, "claude-code", "wrong").await;
    assert_eq!(status, 401);
}

/// **默认关**。装 hook 是一个决定,把整段对话送上网是另一个决定,后者不能靠前者
/// 顺带同意。
///
/// 而且关着时的回答是 `reason: disabled` 而**不是** `unreadable` —— 这证明门控排在
/// 读文件之前:下面那个 transcript 路径是真实存在、可读的。
#[tokio::test]
async fn disabled_by_default_and_gate_precedes_the_read() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("real.jsonl");
    std::fs::write(
        &p,
        r#"{"type":"assistant","uuid":"m0","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#,
    )
    .unwrap();

    let addr = server(false).await;
    seed(addr, Some(&p.to_string_lossy())).await;
    let (status, v) = messages(addr, "claude-code", "tok").await;
    assert_eq!(status, 200, "「你没开」不是错误");
    assert_eq!(v["source"], "terminal");
    assert_eq!(v["reason"], "disabled", "门控没有排在读文件之前");
    assert!(v["messages"].is_null(), "回落时不该返回空消息列表");
}

#[tokio::test]
async fn missing_transcript_path_falls_back() {
    let addr = server(true).await;
    seed(addr, None).await;
    let (_, v) = messages(addr, "claude-code", "tok").await;
    assert_eq!(v["source"], "terminal");
    assert_eq!(v["reason"], "no-transcript-path");
}

#[tokio::test]
async fn an_unreadable_file_falls_back() {
    let addr = server(true).await;
    seed(addr, Some("/definitely/not/here.jsonl")).await;
    let (_, v) = messages(addr, "claude-code", "tok").await;
    assert_eq!(v["source"], "terminal");
    assert_eq!(v["reason"], "unreadable");
}

/// C1 只做 Claude。别家要说清楚是「还不支持」,不是「出错了」。
#[tokio::test]
async fn an_agent_without_transcript_support_falls_back() {
    let addr = server(true).await;
    reqwest::Client::new()
        .post(format!("http://{addr}/hook/codex/session-start"))
        .header("Authorization", format!("Bearer {}", hook_token()))
        .header("X-Octoterm-Session", "1")
        .header("Content-Type", "application/json")
        .body(r#"{"session_id":"s1"}"#)
        .send()
        .await
        .unwrap();
    let (_, v) = messages(addr, "codex", "tok").await;
    assert_eq!(v["source"], "terminal");
    assert_eq!(v["reason"], "unsupported-agent");
}

#[tokio::test]
async fn an_unknown_agent_session_is_404() {
    let addr = server(true).await;
    let (status, _) = messages(addr, "claude-code", "tok").await;
    assert_eq!(status, 404);
}

/// 正常路径:读得到就返回消息 + 游标,并且第二次带游标回来是空的。
#[tokio::test]
async fn reads_and_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("t.jsonl");
    std::fs::write(
        &p,
        r#"{"type":"assistant","uuid":"m0","message":{"role":"assistant","content":[{"type":"text","text":"hello"}]}}
"#,
    )
    .unwrap();

    let addr = server(true).await;
    seed(addr, Some(&p.to_string_lossy())).await;
    let (status, v) = messages(addr, "claude-code", "tok").await;
    assert_eq!(status, 200);
    assert_eq!(v["source"], "transcript");
    assert_eq!(v["messages"][0]["id"], "m0");
    assert_eq!(v["messages"][0]["blocks"][0]["kind"], "text");
    assert_eq!(v["reset"], true);

    let cursor = v["cursor"].as_str().unwrap();
    let r = reqwest::Client::new()
        .get(format!(
            "http://{addr}/api/agents/messages?agent_id=claude-code&agent_session_id=s1&after={cursor}"
        ))
        .header("Authorization", "Bearer tok")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let v2: serde_json::Value = serde_json::from_str(&r).unwrap();
    assert!(v2["messages"].as_array().unwrap().is_empty(), "没有新内容却返回了消息");
    assert_eq!(v2["reset"], false);
}
