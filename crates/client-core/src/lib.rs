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

#[derive(Default)]
pub struct ResumeTracker {
    last_seq: Option<u64>,
}

impl ResumeTracker {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn on_data(&mut self, end_seq: u64) {
        self.last_seq = Some(end_seq);
    }
    pub fn on_resync_end(&mut self, seq: u64) {
        self.last_seq = Some(seq);
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
    fn resume_tracker_follows_stream() {
        let mut t = ResumeTracker::new();
        assert_eq!(t.last_seq(), None);
        t.on_data(100);
        assert_eq!(t.last_seq(), Some(100));
        t.on_resync_end(500);
        assert_eq!(t.last_seq(), Some(500));
    }
}
