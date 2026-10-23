mod common;

use octoterm_server::agent::claude_transcript::parse;
use octoterm_server::agent::store::hook_token;
use octoterm_server::agent::transcript::{read_older_window, read_window, Block, Message, Role, MAX_MESSAGES, MAX_RESPONSE_BYTES};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

fn line(id: usize, text: &str) -> String {
    format!("{}\n", json!({"type":"user", "uuid":format!("m{id}"),
        "message":{"role":"user", "content":text}}))
}

fn all_pages(path: &Path, parse: &dyn Fn(&str, u64) -> Vec<Message>) -> Vec<Message> {
    let first = read_window(path, None, parse).unwrap();
    let mut result = first.messages;
    let mut cursor = first.older_cursor;
    let mut visited = HashSet::new();
    while let Some(before) = cursor {
        assert!(visited.insert(before.clone()), "backward cursor stalled");
        assert!(visited.len() < 100, "pagination did not reach the beginning");
        let page = read_older_window(path, &before, parse).unwrap();
        assert!(!page.reset);
        assert!(page.messages.len() <= MAX_MESSAGES);
        assert!(serde_json::to_vec(&page.messages).unwrap().len() <= MAX_RESPONSE_BYTES);
        cursor = page.older_cursor;
        let mut earlier = page.messages;
        earlier.extend(result);
        result = earlier;
    }
    result
}

#[test]
fn pages_recover_every_message_across_byte_and_count_caps_and_utf8_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let text = "历史🙂".repeat(700);
    std::fs::write(&path, (0..650).map(|i| line(i, &text)).collect::<String>()).unwrap();
    let first = read_window(&path, None, &parse).unwrap();
    assert!(first.older_cursor.is_some());
    assert_ne!(first.messages[0].id, "m0");
    let all = all_pages(&path, &parse);
    assert_eq!(all.iter().map(|m| m.id.clone()).collect::<Vec<_>>(), (0..650).map(|i| format!("m{i}")).collect::<Vec<_>>());
}

#[test]
fn paging_inside_one_record_does_not_skip_messages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    std::fs::write(&path, "{\"messages\":650}\n").unwrap();
    let parse = |text: &str, _: u64| -> Vec<Message> {
        let count = serde_json::from_str::<Value>(text.trim()).unwrap()["messages"].as_u64().unwrap();
        (0..count).map(|i| Message { id: format!("m{i}"), role: Role::User, ts: None,
            blocks: vec![Block::Text { text: i.to_string() }] }).collect()
    };
    assert_eq!(all_pages(&path, &parse).len(), 650);
}

#[test]
fn backward_pages_do_not_move_the_live_forward_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    std::fs::write(&path, (0..450).map(|i| line(i, "hi")).collect::<String>()).unwrap();
    let first = read_window(&path, None, &parse).unwrap();
    let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(line(450, "new message").as_bytes()).unwrap(); file.flush().unwrap();
    let older = read_older_window(&path, first.older_cursor.as_ref().unwrap(), &parse).unwrap();
    assert!(!older.reset);
    assert_eq!(older.messages.last().unwrap().id, "m249");
    let live = read_window(&path, Some(&first.cursor), &parse).unwrap();
    assert_eq!(live.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["m450"]);
}

#[test]
fn rewritten_ranges_and_malformed_cursors_reset_the_timeline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let text = (0..450).map(|i| line(i, "hi")).collect::<String>();
    std::fs::write(&path, &text).unwrap();
    let first = read_window(&path, None, &parse).unwrap();
    std::fs::write(&path, text.replace("m0", "n0")).unwrap();
    assert!(read_older_window(&path, first.older_cursor.as_ref().unwrap(), &parse).unwrap().reset);
    assert!(read_older_window(&path, "not-a-cursor", &parse).unwrap().reset);
    std::fs::write(&path, line(0, "compacted")).unwrap();
    let reset = read_older_window(&path, first.older_cursor.as_ref().unwrap(), &parse).unwrap();
    assert!(reset.reset);
    assert_eq!(reset.messages.len(), 1);
    assert!(reset.older_cursor.is_none());
}

#[test]
fn metadata_only_ranges_still_reach_the_first_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let metadata = format!("{}\n", json!({"type":"metadata", "padding":"x".repeat(100_000)}));
    let text = line(0, "first prompt") + &metadata.repeat(90) + &line(1, "latest prompt");
    std::fs::write(&path, text).unwrap();
    let all = all_pages(&path, &parse);
    assert_eq!(all.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["m0", "m1"]);
}

#[test]
fn changing_a_fully_displayed_range_invalidates_its_backward_anchor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let metadata = format!("{}\n", json!({"type":"metadata", "padding":"x".repeat(100_000)}));
    let text = line(0, "first") + &metadata.repeat(50) + &line(1, "latest");
    std::fs::write(&path, &text).unwrap();
    let first = read_window(&path, None, &parse).unwrap();
    assert_eq!(first.messages.len(), 1);
    std::fs::write(&path, text.replace("latest", "edited")).unwrap();
    let reset = read_older_window(&path, first.older_cursor.as_ref().unwrap(), &parse).unwrap();
    assert!(reset.reset);
}

#[tokio::test]
async fn messages_endpoint_pages_to_the_first_prompt_and_rejects_mixed_directions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    std::fs::write(&path, (0..450).map(|i| line(i, "hello")).collect::<String>()).unwrap();
    let addr = common::start_test_server_at("tok", 1 << 16, Default::default(), vec![]).await;
    let client = reqwest::Client::new();
    client.post(format!("http://{addr}/hook/claude-code/session-start"))
        .bearer_auth(hook_token()).header("X-Octoterm-Session", "1")
        .json(&json!({"session_id":"s1", "transcript_path":path})).send().await.unwrap();
    let url = format!("http://{addr}/api/agents/messages?agent_id=claude-code&agent_session_id=s1");
    let mut page: Value = client.get(&url).bearer_auth("tok").send().await.unwrap().json().await.unwrap();
    let mut ids: Vec<String> = page["messages"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_string()).collect();
    while let Some(before) = page["older_cursor"].as_str() {
        page = client.get(&url).query(&[("before", before)]).bearer_auth("tok").send().await.unwrap().json().await.unwrap();
        let mut earlier: Vec<String> = page["messages"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_string()).collect();
        earlier.extend(ids); ids = earlier;
    }
    assert_eq!(ids, (0..450).map(|i| format!("m{i}")).collect::<Vec<_>>());
    let bad = client.get(&url).query(&[("before", "x"), ("after", "y")]).bearer_auth("tok").send().await.unwrap();
    assert_eq!(bad.status(), 400);
    assert_eq!(client.get(&url).query(&[("before", "x")]).send().await.unwrap().status(), 401);
}
