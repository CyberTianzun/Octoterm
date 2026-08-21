mod common;
use common::{control, parse_server, start_test_server};

use futures_util::{SinkExt, StreamExt};
use octoterm_protocol::{ClientMsg, ServerMsg, CONTROL_CHANNEL, PROTO_VERSION};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn good_token_gets_hello_ok() {
    let url = start_test_server("s3cret").await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws.send(control(&ClientMsg::Hello { token: "s3cret".into(), proto: PROTO_VERSION }))
        .await
        .unwrap();
    let (ch, msg) = parse_server(ws.next().await.unwrap().unwrap()).unwrap();
    assert_eq!(ch, CONTROL_CHANNEL);
    assert_eq!(msg.unwrap(), ServerMsg::HelloOk { proto: PROTO_VERSION });
}

#[tokio::test]
async fn bad_token_is_rejected_and_closed() {
    let url = start_test_server("s3cret").await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws.send(control(&ClientMsg::Hello { token: "wrong".into(), proto: PROTO_VERSION }))
        .await
        .unwrap();
    let (_, msg) = parse_server(ws.next().await.unwrap().unwrap()).unwrap();
    assert!(matches!(msg.unwrap(), ServerMsg::Error { .. }));
    // 之后连接应关闭
    loop {
        match ws.next().await {
            None => break,
            Some(Ok(Message::Close(_))) => break,
            Some(Err(_)) => break,
            Some(Ok(_)) => continue,
        }
    }
}

#[tokio::test]
async fn explicit_config_path_must_exist() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nope.toml");
    assert!(octoterm_server::config::Config::load(Some(path.clone())).is_err());
    assert!(!path.exists(), "load 不得创建文件");
}

#[tokio::test]
async fn config_load_reads_existing_and_fills_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "listen = \"0.0.0.0:1234\"\ntoken = \"fixed\"\n").unwrap();
    let c = octoterm_server::config::Config::load(Some(path.clone())).unwrap();
    assert_eq!(c.listen.map(|l| l.to_string()).as_deref(), Some("0.0.0.0:1234"));
    assert_eq!(c.token.as_deref(), Some("fixed"));

    // 没写 listen 时**保持 None**,而不是在这一层补一个默认值 —— server 与 desktop
    // 的默认监听不一样(只回环 vs 全网卡),谁用谁定。
    std::fs::write(&path, "token = \"only\"\n").unwrap();
    let c = octoterm_server::config::Config::load(Some(path)).unwrap();
    assert_eq!(c.listen, None, "「没写」不该在 Config 这一层被抹成默认值");
    assert_eq!(c.token.as_deref(), Some("only"));
}

#[tokio::test]
async fn non_binary_first_frame_gets_error_reply() {
    let url = start_test_server("s3cret").await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws.send(Message::Text("hi".into())).await.unwrap();
    let (_, msg) = parse_server(ws.next().await.unwrap().unwrap()).unwrap();
    assert!(matches!(msg.unwrap(), ServerMsg::Error { .. }));
}

#[tokio::test]
async fn serves_embedded_index() {
    let url = start_test_server("t").await; // ws://addr/ws
    let http = url.replace("ws://", "http://").replace("/ws", "/");
    let body = reqwest::get(&http).await.unwrap().text().await.unwrap();
    assert!(body.contains("octoterm"));
}

#[tokio::test]
async fn serves_named_asset_with_correct_mime() {
    let url = start_test_server("t").await;
    let http = url.replace("ws://", "http://").replace("/ws", "/style.css");
    let resp = reqwest::get(&http).await.unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ct.starts_with("text/css"), "got {ct}");
}

#[tokio::test]
async fn fallback_serves_index_html_mime() {
    let url = start_test_server("t").await;
    let http = url.replace("ws://", "http://").replace("/ws", "/missing.png");
    let resp = reqwest::get(&http).await.unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ct.starts_with("text/html"), "got {ct}");
    assert!(resp.text().await.unwrap().contains("octoterm"));
}
