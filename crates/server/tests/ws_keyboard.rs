#![cfg(windows)]

mod common;

use common::{connect, control, parse_server};
use futures_util::{SinkExt, StreamExt};
use octoterm_protocol::{ClientMsg, Frame, ServerMsg};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn win32_keyboard_records_survive_websocket_and_conpty() {
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("windows-key-probe.exe");
    let source = dir.path().join("windows-key-probe.rs");
    std::fs::write(&source, include_str!("fixtures/windows_key_probe.rs")).unwrap();
    let build = std::process::Command::new("rustc")
        .args(["--edition", "2024"])
        .arg(source)
        .arg("-o")
        .arg(&probe)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let url = common::start_test_server("t").await;
    let mut ws = connect(&url).await;
    ws.send(control(&ClientMsg::NewSession {
        name: Some("keyboard probe".into()),
        command: Some(vec![probe.to_string_lossy().into_owned()]),
        cwd: None,
    }))
    .await
    .unwrap();
    let id = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some((0, Ok(ServerMsg::SessionEvent { session, .. }))) =
                parse_server(ws.next().await.unwrap().unwrap())
            {
                break session.id;
            }
        }
    })
    .await
    .unwrap();
    ws.send(control(&ClientMsg::Attach {
        id,
        channel: 1,
        last_seq: None,
        cols: 160,
        rows: 24,
    }))
    .await
    .unwrap();

    let records = [
        (13, 28, 13, 16), // Shift+Enter
        (74, 36, 10, 8),  // Ctrl+J: retain J despite the LF character
        (9, 15, 9, 8),    // Ctrl+Tab
        (38, 72, 0, 258), // Alt+Up, enhanced key
        (33, 73, 0, 272), // Shift+PageUp, not local scrolling
        (112, 59, 0, 24), // Ctrl+Shift+F1
    ];
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut output = Vec::new();
        let mut sent = false;
        while let Some(msg) = ws.next().await {
            if let Some((1, Err(bytes))) = parse_server(msg.unwrap()) {
                output.extend_from_slice(&bytes);
                let text = String::from_utf8_lossy(&output);
                if !sent && text.contains("KEYBOARD_READY") {
                    // A baseline demonstrates how the old VT Ctrl+J becomes Ctrl+Enter.
                    let mut input = String::from("\n");
                    for (vk, scan, ch, state) in records {
                        for down in [1, 0] {
                            input.push_str(&format!("\x1b[{vk};{scan};{ch};{down};{state};1_"));
                        }
                    }
                    ws.send(Message::Binary(
                        Frame::new(1, input.into_bytes()).encode().into(),
                    ))
                    .await
                    .unwrap();
                    sent = true;
                }
                if text.contains("KEY down=0 vk=112 scan=59 char=0 state=24 repeat=1") {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&output).into_owned()
    })
    .await;
    // Release the native reader even if input collection times out.
    ws.send(control(&ClientMsg::KillSession { id }))
        .await
        .unwrap();
    let output = result.expect("native keyboard probe timed out");
    assert!(
        output.contains("KEY down=1 vk=13 scan=28 char=10 state=8 repeat=1"),
        "{output}"
    );
    for (vk, scan, ch, state) in records {
        for down in [1, 0] {
            let expected =
                format!("KEY down={down} vk={vk} scan={scan} char={ch} state={state} repeat=1");
            assert!(output.contains(&expected), "missing {expected}: {output}");
        }
    }
}
