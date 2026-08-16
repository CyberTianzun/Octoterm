use std::collections::VecDeque;

pub struct SeqBuffer {
    cap: usize,
    buf: VecDeque<u8>,
    end_seq: u64,
}

impl SeqBuffer {
    pub fn new(cap: usize) -> Self {
        Self { cap, buf: VecDeque::with_capacity(cap), end_seq: 0 }
    }

    pub fn append(&mut self, bytes: &[u8]) {
        self.end_seq += bytes.len() as u64;
        if bytes.len() >= self.cap {
            self.buf.clear();
            self.buf.extend(&bytes[bytes.len() - self.cap..]);
            return;
        }
        let overflow = (self.buf.len() + bytes.len()).saturating_sub(self.cap);
        self.buf.drain(..overflow);
        self.buf.extend(bytes);
    }

    pub fn end_seq(&self) -> u64 {
        self.end_seq
    }

    pub fn start_seq(&self) -> u64 {
        self.end_seq - self.buf.len() as u64
    }

    pub fn read_from(&self, seq: u64) -> Option<Vec<u8>> {
        if seq < self.start_seq() || seq > self.end_seq {
            return None;
        }
        let offset = (seq - self.start_seq()) as usize;
        Some(self.buf.iter().skip(offset).copied().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_advances_seq_and_reads_back() {
        let mut b = SeqBuffer::new(16);
        b.append(b"hello");
        assert_eq!(b.end_seq(), 5);
        assert_eq!(b.read_from(0).unwrap(), b"hello");
        assert_eq!(b.read_from(3).unwrap(), b"lo");
        assert_eq!(b.read_from(5).unwrap(), b"");
    }

    #[test]
    fn eviction_keeps_seq_monotonic() {
        let mut b = SeqBuffer::new(4);
        b.append(b"abcdef"); // 只保留最后 4 字节 "cdef"
        assert_eq!(b.end_seq(), 6);
        assert_eq!(b.start_seq(), 2);
        assert_eq!(b.read_from(0), None); // 太旧
        assert_eq!(b.read_from(2).unwrap(), b"cdef");
    }

    #[test]
    fn oversized_append_keeps_tail() {
        let mut b = SeqBuffer::new(4);
        b.append(b"0123456789");
        assert_eq!(b.end_seq(), 10);
        assert_eq!(b.read_from(6).unwrap(), b"6789");
        assert_eq!(b.read_from(5), None);
    }
}
