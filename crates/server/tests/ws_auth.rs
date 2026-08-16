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
async fn config_generates_and_persists_token() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let c1 = octoterm_server::config::Config::load_or_init(Some(path.clone())).unwrap();
    assert_eq!(c1.listen.to_string(), "127.0.0.1:7683");
    assert!(!c1.token.is_empty());
    let c2 = octoterm_server::config::Config::load_or_init(Some(path)).unwrap();
    assert_eq!(c1.token, c2.token);
}

#[tokio::test]
async fn non_binary_first_frame_gets_error_reply() {
    let url = start_test_server("s3cret").await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws.send(Message::Text("hi".into())).await.unwrap();
    let (_, msg) = parse_server(ws.next().await.unwrap().unwrap()).unwrap();
    assert!(matches!(msg.unwrap(), ServerMsg::Error { .. }));
}

#[cfg(unix)]
#[tokio::test]
async fn generated_config_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    octoterm_server::config::Config::load_or_init(Some(path.clone())).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
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
