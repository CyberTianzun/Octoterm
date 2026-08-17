//! `GET /api/launchers`:鉴权、返回结构、以及"用户 config.toml 里的自定义项
//! 一定出现在里面"。
//!
//! 系统上装没装 iTerm2 / Windows Terminal 是环境相关的,这里不断言 —— 那部分
//! 的解析逻辑由 launcher 模块的单测覆盖(纯函数,不碰文件系统)。

mod common;

use octoterm_server::config::LauncherSpec;
use serde_json::Value;

async fn get(addr: std::net::SocketAddr, token: Option<&str>) -> reqwest::Response {
    let mut req = reqwest::Client::new().get(format!("http://{addr}/api/launchers"));
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    req.send().await.unwrap()
}

async fn get_json(addr: std::net::SocketAddr, token: &str) -> Value {
    let body = get(addr, Some(token)).await.text().await.unwrap();
    serde_json::from_str(&body).unwrap()
}

#[tokio::test]
async fn requires_the_same_bearer_token_as_the_websocket() {
    let addr = common::start_test_server_at("t", 1 << 20, Default::default(), Vec::new()).await;

    assert_eq!(get(addr, None).await.status(), 401, "无 Authorization 头");
    assert_eq!(get(addr, Some("wrong")).await.status(), 401, "token 不对");
    assert_eq!(get(addr, Some("t")).await.status(), 200);
}

#[tokio::test]
async fn always_returns_at_least_the_builtin_shell() {
    let addr = common::start_test_server_at("t", 1 << 20, Default::default(), Vec::new()).await;
    let body = get_json(addr, "t").await;

    let list = body["launchers"].as_array().expect("launchers 必须是数组");
    let first = &list[0];
    assert_eq!(first["provider"], "builtin", "内置默认永远排第一");
    assert!(!first["command"].as_array().unwrap().is_empty());
    assert!(first["id"].as_str().unwrap().starts_with("builtin:"));
    // 每一条都得能直接拿去 spawn
    for l in list {
        assert!(!l["command"].as_array().unwrap().is_empty(), "{l}");
        assert!(!l["name"].as_str().unwrap().is_empty(), "{l}");
    }
}

#[tokio::test]
async fn config_launchers_show_up_after_the_builtin_one() {
    let specs = vec![LauncherSpec {
        name: "prod ssh".into(),
        command: vec!["ssh".into(), "prod01".into()],
        cwd: Some("/tmp".into()),
    }];
    let addr = common::start_test_server_at("t", 1 << 20, Default::default(), specs).await;
    let body = get_json(addr, "t").await;
    let list = body["launchers"].as_array().unwrap();

    let mine = list.iter().find(|l| l["id"] == "config:prod ssh").expect("自定义项应该在列表里");
    assert_eq!(mine["command"], serde_json::json!(["ssh", "prod01"]));
    assert_eq!(mine["cwd"], "/tmp");
    assert_eq!(list[0]["provider"], "builtin", "内置的仍然排在前面");
}
