mod common;
use common::{control, parse_server, start_test_server};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use octoterm_protocol::{ClientMsg, ServerMsg, SessionEventKind, PROTO_VERSION};

async fn connect(url: &str) -> impl futures_util::Sink<tokio_tungstenite::tungstenite::Message, Error = tokio_tungstenite::tungstenite::Error>
       + futures_util::Stream<Item = Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>>
       + Unpin {
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws.send(control(&ClientMsg::Hello { token: "t".into(), proto: PROTO_VERSION })).await.unwrap();
    let (_, msg) = parse_server(ws.next().await.unwrap().unwrap()).unwrap();
    assert!(matches!(msg.unwrap(), ServerMsg::HelloOk { .. }));
    ws
}

async fn next_control(ws: &mut (impl futures_util::Stream<Item = Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>> + Unpin)) -> ServerMsg {
    loop {
        let msg = ws.next().await.unwrap().unwrap();
        if let Some((0, Ok(m))) = parse_server(msg) {
            return m;
        }
    }
}

#[tokio::test]
async fn session_crud_over_ws() {
    let url = start_test_server("t").await;
    let mut ws = connect(&url).await;

    // 创建:先收到事件推送,再收到列表响应(create 内部先 emit 事件)
    ws.send(control(&ClientMsg::NewSession { name: Some("alpha".into()), command: None }))
        .await
        .unwrap();
    let evt = next_control(&mut ws).await;
    let id = match evt {
        ServerMsg::SessionEvent { event: SessionEventKind::Created, session } => session.id,
        other => panic!("expected created event, got {other:?}"),
    };

    ws.send(control(&ClientMsg::ListSessions)).await.unwrap();
    match next_control(&mut ws).await {
        ServerMsg::Sessions { sessions } => {
            assert_eq!(sessions.len(), 1);
            assert_eq!(sessions[0].name, "alpha");
        }
        other => panic!("unexpected: {other:?}"),
    }

    ws.send(control(&ClientMsg::RenameSession { id, name: "beta".into() })).await.unwrap();
    match next_control(&mut ws).await {
        ServerMsg::SessionEvent { event: SessionEventKind::Renamed, session } => {
            assert_eq!(session.name, "beta")
        }
        other => panic!("unexpected: {other:?}"),
    }

    ws.send(control(&ClientMsg::KillSession { id })).await.unwrap();
    loop {
        if let ServerMsg::SessionEvent { event: SessionEventKind::Closed, .. } =
            next_control(&mut ws).await
        {
            break;
        }
    }
}

#[tokio::test]
async fn preview_returns_base64_repaint() {
    let url = start_test_server("t").await;
    let mut ws = connect(&url).await;
    #[cfg(unix)]
    let command = Some(vec!["/bin/sh".into(), "-c".into(), "printf PREVIEW_MARK; sleep 30".into()]);
    #[cfg(windows)]
    let command = Some(vec!["powershell.exe".into(), "-Command".into(), "Write-Host PREVIEW_MARK; Start-Sleep 30".into()]);

    ws.send(control(&ClientMsg::NewSession { name: None, command })).await.unwrap();
    let id = match next_control(&mut ws).await {
        ServerMsg::SessionEvent { session, .. } => session.id,
        other => panic!("unexpected: {other:?}"),
    };
    tokio::time::sleep(std::time::Duration::from_millis(500)).await; // 等输出进 grid

    ws.send(control(&ClientMsg::Preview { id })).await.unwrap();
    match next_control(&mut ws).await {
        ServerMsg::PreviewData { id: got, data } => {
            assert_eq!(got, id);
            let bytes = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
            assert!(String::from_utf8_lossy(&bytes).contains("PREVIEW_MARK"));
        }
        other => panic!("unexpected: {other:?}"),
    }
    ws.send(control(&ClientMsg::KillSession { id })).await.unwrap();
}

#[tokio::test]
async fn unknown_session_errors() {
    let url = start_test_server("t").await;
    let mut ws = connect(&url).await;
    ws.send(control(&ClientMsg::Preview { id: 999 })).await.unwrap();
    assert!(matches!(next_control(&mut ws).await, ServerMsg::Error { .. }));
}
