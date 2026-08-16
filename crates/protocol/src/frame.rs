use thiserror::Error;

pub const CONTROL_CHANNEL: u32 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub channel: u32,
    pub flags: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("frame shorter than 5-byte header")]
    TooShort,
}

impl Frame {
    pub fn new(channel: u32, payload: Vec<u8>) -> Self {
        Self { channel, flags: 0, payload }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(5 + self.payload.len());
        buf.extend_from_slice(&self.channel.to_le_bytes());
        buf.push(self.flags);
        buf.extend_from_slice(&self.payload);
        buf
    }

    pub fn decode(data: &[u8]) -> Result<Frame, FrameError> {
        if data.len() < 5 {
            return Err(FrameError::TooShort);
        }
        Ok(Frame {
            channel: u32::from_le_bytes(data[0..4].try_into().unwrap()),
            flags: data[4],
            payload: data[5..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let f = Frame::new(7, b"hello".to_vec());
        let bytes = f.encode();
        assert_eq!(bytes.len(), 5 + 5);
        assert_eq!(&bytes[0..4], &7u32.to_le_bytes());
        assert_eq!(bytes[4], 0);
        assert_eq!(Frame::decode(&bytes).unwrap(), f);
    }

    #[test]
    fn empty_payload_ok() {
        let f = Frame::new(0, vec![]);
        assert_eq!(Frame::decode(&f.encode()).unwrap(), f);
    }

    #[test]
    fn too_short_rejected() {
        assert_eq!(Frame::decode(&[1, 2, 3]), Err(FrameError::TooShort));
    }
}
