//! 读 transcript 的窗口与游标。
//!
//! 这部分**与 agent 无关** —— 三家的文件都是「一行一条、只追加」,只有「一行是什么
//! 意思」属于各自的 adapter。C3 加 Codex / Grok 时这里一行都不用改。

use octoterm_server::agent::transcript::{plan_read, Plan, INCREMENT_BYTES, WINDOW_BYTES};

/// 第一屏要的是**最近**发生的事,不是这个会话最早的几句。
#[test]
fn first_read_takes_the_tail_not_the_head() {
    let len = WINDOW_BYTES * 3;
    let p = plan_read(len, None);
    assert_eq!(p.start, len - WINDOW_BYTES);
    assert!(p.reset, "首次读是整窗替换");
}

#[test]
fn a_small_file_is_read_whole() {
    let p = plan_read(100, None);
    assert_eq!(p.start, 0);
    assert!(p.reset);
}

#[test]
fn cursor_resumes_exactly_where_it_left_off() {
    let p = plan_read(5000, Some((1234, 5000)));
    assert_eq!(p.start, 1234);
    assert!(!p.reset, "续读是追加,不是替换");
}

/// compact 之后、或者换了个会话,文件会变小甚至整个换掉。
/// 这时旧游标指向的位置已经不是原来那条消息了 —— 必须整窗重发,让客户端整段替换,
/// 否则会把新文件的中段当成旧文件的续集接上去。
#[test]
fn a_shrunk_file_invalidates_the_cursor() {
    let p = plan_read(500, Some((1234, 5000)));
    assert_eq!(p.start, 0);
    assert!(p.reset, "文件变小了却还在追加");
}

/// 游标偏移落在文件长度之外,同样只能重来。
#[test]
fn an_out_of_range_cursor_invalidates() {
    let p = plan_read(1000, Some((9999, 1000)));
    assert!(p.reset);
}

/// 增量一次最多读一段。读不完不是丢弃,是**让客户端再拉一次** ——
/// 增量里丢消息就是静默丢数据。
#[test]
fn an_increment_is_capped_by_bytes_and_reports_more() {
    let len = INCREMENT_BYTES * 4;
    let p = plan_read(len, Some((0, len)));
    assert_eq!(p.start, 0);
    assert_eq!(p.end, INCREMENT_BYTES, "增量一次读一段");
    assert!(p.more, "还有剩的要告诉客户端");
    assert!(!p.reset);
}

#[test]
fn an_increment_that_fits_reports_no_more() {
    let p = plan_read(1000, Some((900, 1000)));
    assert_eq!((p.start, p.end), (900, 1000));
    assert!(!p.more);
}

/// 没有新字节时不该产生一次空读。
#[test]
fn nothing_new_is_an_empty_plan() {
    let p = plan_read(1000, Some((1000, 1000)));
    assert_eq!(p.start, p.end);
    assert!(!p.more);
}

/// 窗口按字节切,起点几乎必然落在半行上。那半行必须丢掉,
/// 否则第一条消息是残的 —— 而残行解析失败后,用户看到的是「少了一条」。
#[test]
fn a_mid_file_window_drops_the_partial_first_line() {
    let text = "AAAA\nBBBB\nCCCC\n";
    // 从 'B' 那一行的中间开始
    let (body, consumed) = octoterm_server::agent::transcript::slice_window(text.as_bytes(), 6, true);
    assert_eq!(std::str::from_utf8(body).unwrap(), "CCCC\n", "半行没被丢掉");
    assert_eq!(consumed, text.len() as u64 - 6);
}

/// 续读的起点本来就是行首(上一次的游标就是这么算的),不该再丢一行。
#[test]
fn a_resumed_window_keeps_its_first_line() {
    let text = "AAAA\nBBBB\n";
    let (body, _) = octoterm_server::agent::transcript::slice_window(text.as_bytes(), 5, false);
    assert_eq!(std::str::from_utf8(body).unwrap(), "BBBB\n");
}

/// 末尾那半行是「正在被写入」的一条,这次不能算数 —— 游标要停在它前面,
/// 下次连着后半截一起读。
#[test]
fn a_trailing_partial_line_is_not_consumed() {
    let text = "AAAA\nBBB";
    let (body, consumed) = octoterm_server::agent::transcript::slice_window(text.as_bytes(), 0, false);
    assert_eq!(std::str::from_utf8(body).unwrap(), "AAAA\n");
    assert_eq!(consumed, 5);
}

/* ---------- 与真实文件接起来 ---------- */

use octoterm_server::agent::claude_transcript::parse;
use octoterm_server::agent::transcript::{decode_cursor, read_window};
use std::io::Write;

fn line(id: &str, text: &str) -> String {
    format!(
        r#"{{"type":"assistant","uuid":"{id}","message":{{"role":"assistant","content":[{{"type":"text","text":"{text}"}}]}}}}"#
    ) + "\n"
}

#[test]
fn reads_then_resumes_without_repeating_or_losing() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("t.jsonl");
    let mut f = std::fs::File::create(&p).unwrap();
    for i in 0..3 {
        f.write_all(line(&format!("m{i}"), &format!("hello {i}")).as_bytes()).unwrap();
    }
    f.flush().unwrap();

    let w = read_window(&p, None, &parse).unwrap();
    assert!(w.reset);
    assert_eq!(w.messages.len(), 3);

    // 没有新内容:空窗,游标不动
    let w2 = read_window(&p, Some(&w.cursor), &parse).unwrap();
    assert!(w2.messages.is_empty());
    assert!(!w2.reset);

    // 追加两条,只应当读到这两条
    let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
    f.write_all(line("m3", "hello 3").as_bytes()).unwrap();
    f.write_all(line("m4", "hello 4").as_bytes()).unwrap();
    f.flush().unwrap();

    let w3 = read_window(&p, Some(&w2.cursor), &parse).unwrap();
    assert!(!w3.reset, "追加不该触发整窗替换");
    assert_eq!(w3.messages.len(), 2, "续读只该拿到新增的那两条");
    assert_eq!(w3.messages[0].id, "m3");
}

/// 正在被写入的那半行:这次不能算数,下次连着后半截一起读。
#[test]
fn a_half_written_line_is_picked_up_on_the_next_read() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("t.jsonl");
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(line("m0", "done").as_bytes()).unwrap();
    let half = line("m1", "half");
    f.write_all(&half.as_bytes()[..20]).unwrap(); // 只写一半
    f.flush().unwrap();

    let w = read_window(&p, None, &parse).unwrap();
    assert_eq!(w.messages.len(), 1, "半行不该被当成一条消息");

    let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
    f.write_all(&half.as_bytes()[20..]).unwrap();
    f.flush().unwrap();

    let w2 = read_window(&p, Some(&w.cursor), &parse).unwrap();
    assert_eq!(w2.messages.len(), 1, "补齐之后应当读到完整的那一条");
    assert_eq!(w2.messages[0].id, "m1");
}

/// compact 之后文件变小:必须整窗重发,客户端整段替换。
#[test]
fn a_compacted_file_forces_a_reset() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("t.jsonl");
    std::fs::write(&p, (0..10).map(|i| line(&format!("m{i}"), "x")).collect::<String>()).unwrap();
    let w = read_window(&p, None, &parse).unwrap();
    assert_eq!(w.messages.len(), 10);

    std::fs::write(&p, line("n0", "after compact")).unwrap();
    let w2 = read_window(&p, Some(&w.cursor), &parse).unwrap();
    assert!(w2.reset, "文件变小了却当成追加");
    assert_eq!(w2.messages.len(), 1);
    assert_eq!(w2.messages[0].id, "n0");
}

#[test]
fn cursor_round_trips_and_rejects_garbage() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("t.jsonl");
    std::fs::write(&p, line("m0", "x")).unwrap();
    let w = read_window(&p, None, &parse).unwrap();
    assert!(decode_cursor(&w.cursor).is_some());
    assert!(decode_cursor("garbage").is_none());
    // 坏游标当成没有游标 —— 整窗重来,而不是报错
    let w2 = read_window(&p, Some("garbage"), &parse).unwrap();
    assert!(w2.reset);
}
