use std::time::Duration;

pub struct Backoff {
    next: Duration,
}

impl Backoff {
    pub fn new() -> Self {
        Self { next: Duration::from_millis(250) }
    }
    pub fn next_delay(&mut self) -> Duration {
        let current = self.next;
        self.next = (self.next * 2).min(Duration::from_secs(10));
        current
    }
    pub fn reset(&mut self) {
        self.next = Duration::from_millis(250);
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

/// seq 记账不变式(与 crates/protocol 的 `ServerMsg::Attached`/`ResyncEnd`
/// 文档一致):数据帧是裸字节,不带 seq。客户端只能从控制消息锚定
/// `last_seq` —— 要么是 `Attached{mode: Replay}` 时自己发出的 `last_seq`
/// (重放字节从该点续接,不需要、也不应该用 `Attached.seq` 去覆盖它;那只是
/// "重放会在哪个 seq 结束"的参考信息),要么是 `ResyncEnd.seq`(resync 的
/// 重绘字节是合成的,不计入账本)。锚定之后,每收到 n 字节数据帧,
/// `last_seq += n`;未锚定前收到的数据帧不计数(`advance` 是 no-op)。
#[derive(Default)]
pub struct ResumeTracker {
    last_seq: Option<u64>,
}

impl ResumeTracker {
    pub fn new() -> Self {
        Self::default()
    }
    /// 收到一条数据帧,按字节长度推进账本;锚定之前是 no-op。
    pub fn advance(&mut self, byte_len: usize) {
        if let Some(seq) = &mut self.last_seq {
            *seq += byte_len as u64;
        }
    }
    /// `ResyncEnd.seq` 锚点。
    pub fn on_resync_end(&mut self, seq: u64) {
        self.last_seq = Some(seq);
    }
    /// `Attached{mode: Replay}` 锚点:锚定到本端自己发出的 resume_point
    /// (即那次 attach 请求里的 `last_seq`),而不是 `Attached.seq`。
    pub fn on_attach_replay(&mut self, resume_point: u64) {
        self.last_seq = Some(resume_point);
    }
    pub fn last_seq(&self) -> Option<u64> {
        self.last_seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let mut b = Backoff::new();
        assert_eq!(b.next_delay(), Duration::from_millis(250));
        assert_eq!(b.next_delay(), Duration::from_millis(500));
        assert_eq!(b.next_delay(), Duration::from_millis(1000));
        for _ in 0..10 {
            b.next_delay();
        }
        assert_eq!(b.next_delay(), Duration::from_secs(10));
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_millis(250));
    }

    #[test]
    fn resume_tracker_ignores_data_before_anchoring() {
        let mut t = ResumeTracker::new();
        assert_eq!(t.last_seq(), None);
        // 数据帧没有 seq;未锚定前收到的字节不该凭空产生一个 last_seq。
        t.advance(100);
        assert_eq!(t.last_seq(), None);
    }

    #[test]
    fn resume_tracker_anchors_on_resync_end_then_advances_by_bytes() {
        let mut t = ResumeTracker::new();
        t.on_resync_end(500);
        assert_eq!(t.last_seq(), Some(500));
        t.advance(42);
        assert_eq!(t.last_seq(), Some(542));
    }

    #[test]
    fn resume_tracker_anchors_on_attach_replay_to_the_resume_point_sent() {
        let mut t = ResumeTracker::new();
        // 重放模式下要锚定到本端自己发出的 last_seq(重连恢复点),而不是
        // Attached.seq(那只是重放结束点的参考信息)。
        t.on_attach_replay(200);
        assert_eq!(t.last_seq(), Some(200));
        t.advance(10);
        assert_eq!(t.last_seq(), Some(210));
    }
}
