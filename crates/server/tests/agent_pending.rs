//! 阻塞式决策:agent 挂在 socket 上等我们写响应,我们等用户拍板。
//!
//! 三条终结路径都必须覆盖 —— 用户回答、超时、**agent 侧断连**。第三条最容易漏,
//! 也最要命:漏了就会留下永远等不到答复的幽灵条目。

mod common;

use octoterm_server::agent::store::hook_token;
use octoterm_server::config::AgentsConfig;
use std::net::SocketAddr;
use std::time::Duration;

fn cfg(timeout_secs: u64) -> AgentsConfig {
    AgentsConfig { pending_timeout_secs: timeout_secs, ..Default::default() }
}

async fn server(timeout_secs: u64) -> SocketAddr {
    common::start_test_server_with_agents("tok", 1 << 16, Default::default(), vec![], cfg(timeout_secs))
        .await
}

fn permission_request(addr: SocketAddr) -> reqwest::RequestBuilder {
    reqwest::Client::new()
        .post(format!("http://{addr}/hook/claude-code/permission-request"))
        .header("Authorization", format!("Bearer {}", hook_token()))
        .header("X-Octoterm-Session", "1")
        .header("Content-Type", "application/json")
        .body(r#"{"session_id":"s1","tool_name":"Bash","tool_input":{"command":"rm -rf /"}}"#)
}

async fn get_json(addr: SocketAddr, path: &str) -> serde_json::Value {
    let t = reqwest::Client::new()
        .get(format!("http://{addr}{path}"))
        .header("Authorization", "Bearer tok")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    serde_json::from_str(&t).unwrap()
}

async fn answer(addr: SocketAddr, id: &str, decision: &str) -> reqwest::StatusCode {
    reqwest::Client::new()
        .post(format!("http://{addr}/api/agents/answer"))
        .header("Authorization", "Bearer tok")
        .header("Content-Type", "application/json")
        .body(format!(r#"{{"pending_id":"{id}","decision":"{decision}"}}"#))
        .send()
        .await
        .unwrap()
        .status()
}

/// 等挂起项出现。轮询而不是 sleep 固定时长 —— 后者要么慢要么脆。
async fn wait_for_pending(addr: SocketAddr) -> String {
    for _ in 0..100 {
        let v = get_json(addr, "/api/agents/pending").await;
        if let Some(p) = v["pending"].as_array().and_then(|a| a.first()) {
            return p["id"].as_str().unwrap().to_string();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("挂起请求一直没出现");
}

#[tokio::test]
async fn user_answer_reaches_the_hook() {
    let addr = server(30).await;
    let hook = tokio::spawn(async move { permission_request(addr).send().await.unwrap() });

    let id = wait_for_pending(addr).await;
    // 挂起期间,会话状态应当是「在等人」,并挂着这个 pending
    let sessions = get_json(addr, "/api/agents/sessions").await;
    assert_eq!(sessions["sessions"][0]["state"], "waiting");
    assert_eq!(sessions["sessions"][0]["pending"], id);

    assert_eq!(answer(addr, &id, "allow").await, 200);

    let body: serde_json::Value =
        serde_json::from_str(&hook.await.unwrap().text().await.unwrap()).unwrap();
    let d = &body["hookSpecificOutput"]["decision"];
    assert_eq!(body["hookSpecificOutput"]["hookEventName"], "PermissionRequest");
    // decision 是**对象**不是字符串 —— 字符串形态实测不生效
    assert_eq!(d["behavior"], "allow");
}

#[tokio::test]
async fn deny_reaches_the_hook() {
    let addr = server(30).await;
    let hook = tokio::spawn(async move { permission_request(addr).send().await.unwrap() });
    let id = wait_for_pending(addr).await;
    assert_eq!(answer(addr, &id, "deny").await, 200);
    let body: serde_json::Value =
        serde_json::from_str(&hook.await.unwrap().text().await.unwrap()).unwrap();
    assert_eq!(body["hookSpecificOutput"]["decision"]["behavior"], "deny");
}

/// R6:重复回答不改变已定的决策,并且要能和「请求不存在」区分开。
#[tokio::test]
async fn second_answer_is_conflict_not_found() {
    let addr = server(30).await;
    let hook = tokio::spawn(async move { permission_request(addr).send().await.unwrap() });
    let id = wait_for_pending(addr).await;
    assert_eq!(answer(addr, &id, "allow").await, 200);
    assert_eq!(answer(addr, &id, "deny").await, 409, "重复提交应当是 409");

    let body: serde_json::Value =
        serde_json::from_str(&hook.await.unwrap().text().await.unwrap()).unwrap();
    assert_eq!(
        body["hookSpecificOutput"]["decision"]["behavior"], "allow",
        "第二次回答不该改变已经发出去的决策"
    );
}

/// 挂起请求必须带着**足够做决定的信息**出来:工具名、命令原文、截止时间。
///
/// 这是线上复盘出来的 —— 第一版横幅只显示「会话名 · 等你回答」,等于让人一键批准
/// 一条看不见的命令。数据一直都在服务端,只是没送出去。
#[tokio::test]
async fn pending_carries_enough_to_decide_on() {
    let addr = server(30).await;
    let hook = tokio::spawn(async move { permission_request(addr).send().await.unwrap() });
    let id = wait_for_pending(addr).await;

    let v = get_json(addr, "/api/agents/pending").await;
    let p = &v["pending"][0];
    assert_eq!(p["tool_name"], "Bash");
    assert_eq!(p["tool_input"]["command"], "rm -rf /", "命令原文必须原样带出来");
    let created = p["created_at"].as_u64().unwrap();
    let expires = p["expires_at"].as_u64().unwrap();
    assert_eq!(expires - created, 30, "截止时间由服务端按配置算好,客户端不该猜");

    // 广播里也要有工具名,这样没拉详情之前列表上就不是一句「等你回答」
    let s = get_json(addr, "/api/agents/sessions").await;
    assert_eq!(s["sessions"][0]["detail"], "Bash");

    answer(addr, &id, "deny").await;
    let _ = hook.await;
}

#[tokio::test]
async fn unknown_pending_is_404() {
    let addr = server(30).await;
    assert_eq!(answer(addr, "nope", "allow").await, 404);
}

#[tokio::test]
async fn unknown_decision_is_400() {
    let addr = server(30).await;
    assert_eq!(answer(addr, "whatever", "maybe").await, 400);
}

/// 没人回答就超时,返回**空对象** = 无决定。Claude 会回落到它自己的审批弹窗 ——
/// 我们绝不代替用户 allow 或 deny。
#[tokio::test]
async fn timeout_falls_back_to_no_decision() {
    let addr = server(1).await;
    let body = permission_request(addr).send().await.unwrap().text().await.unwrap();
    assert_eq!(body, "{}", "超时必须是无决定,不是 deny");
    let v = get_json(addr, "/api/agents/pending").await;
    assert!(v["pending"].as_array().unwrap().is_empty(), "超时后挂起项该被摘掉");
}

/// **本 task 的核心**:agent 侧断开时,axum 直接丢掉 handler 的 future,`await`
/// 之后一行都不会跑。只有 Drop guard 能保证挂起项被摘掉 —— 否则它会一直挂到超时,
/// 客户端上显示一个永远等不到答复的「有事找你」。
#[tokio::test]
async fn agent_disconnect_clears_the_pending_entry() {
    let addr = server(300).await;
    // 给一个远短于 pending 超时的请求超时,让客户端先断
    let client = reqwest::Client::builder().timeout(Duration::from_millis(400)).build().unwrap();
    let req = client
        .post(format!("http://{addr}/hook/claude-code/permission-request"))
        .header("Authorization", format!("Bearer {}", hook_token()))
        .header("X-Octoterm-Session", "1")
        .header("Content-Type", "application/json")
        .body(r#"{"session_id":"s1","tool_name":"Bash"}"#);
    let handle = tokio::spawn(async move { req.send().await });

    let _id = wait_for_pending(addr).await;
    let _ = handle.await.unwrap(); // 客户端超时断开

    for _ in 0..100 {
        let v = get_json(addr, "/api/agents/pending").await;
        if v["pending"].as_array().unwrap().is_empty() {
            let s = get_json(addr, "/api/agents/sessions").await;
            assert!(s["sessions"][0]["pending"].is_null(), "会话上的 pending 也该被清掉");
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("agent 断连后挂起项没有被摘掉 —— Drop guard 没生效");
}
