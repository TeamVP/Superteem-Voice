//! Pin audio frame format. See protocol/PROTOCOL-v0.md.

use thiserror::Error;

pub const VERSION: u8 = 0;
pub const HEADER_LEN: usize = 15;

pub const FLAG_MUTED: u8 = 1 << 0;
pub const FLAG_FLASH_RECORDING: u8 = 1 << 1;
pub const FLAG_LOW_BATTERY: u8 = 1 << 2;
pub const FLAG_BACKFILL: u8 = 1 << 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    RawPcm16 = 0,
    Opus = 1,
    ImaAdpcm = 2,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("frame shorter than its header or declared payload")]
    Truncated,
    #[error("unsupported protocol version {0}")]
    Version(u8),
    #[error("unknown codec {0}")]
    Codec(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub flags: u8,
    pub pin_session_id: u16,
    pub sequence: u32,
    pub sample_counter: u32,
    pub codec: Codec,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.payload.len());
        out.push(VERSION);
        out.push(self.flags);
        out.extend_from_slice(&self.pin_session_id.to_le_bytes());
        out.extend_from_slice(&self.sequence.to_le_bytes());
        out.extend_from_slice(&self.sample_counter.to_le_bytes());
        out.push(self.codec as u8);
        out.extend_from_slice(&(self.payload.len() as u16).to_le_bytes());
        out.extend_from_slice(&self.payload);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Frame, FrameError> {
        if bytes.len() < HEADER_LEN {
            return Err(FrameError::Truncated);
        }
        if bytes[0] != VERSION {
            return Err(FrameError::Version(bytes[0]));
        }
        let codec = match bytes[12] {
            0 => Codec::RawPcm16,
            1 => Codec::Opus,
            2 => Codec::ImaAdpcm,
            other => return Err(FrameError::Codec(other)),
        };
        let len = u16::from_le_bytes([bytes[13], bytes[14]]) as usize;
        if bytes.len() < HEADER_LEN + len {
            return Err(FrameError::Truncated);
        }
        Ok(Frame {
            flags: bytes[1],
            pin_session_id: u16::from_le_bytes([bytes[2], bytes[3]]),
            sequence: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            sample_counter: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            codec,
            payload: bytes[HEADER_LEN..HEADER_LEN + len].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Frame {
        Frame {
            flags: FLAG_LOW_BATTERY,
            pin_session_id: 7,
            sequence: 42,
            sample_counter: 320,
            codec: Codec::Opus,
            payload: vec![1, 2, 3],
        }
    }

    #[test]
    fn round_trips() {
        assert_eq!(Frame::decode(&sample().encode()).unwrap(), sample());
    }

    #[test]
    fn matches_known_layout() {
        let frame = Frame {
            flags: 0,
            pin_session_id: 1,
            sequence: 1,
            sample_counter: 0x140,
            codec: Codec::RawPcm16,
            payload: vec![1, 2],
        };
        let expected = [0, 0, 1, 0, 1, 0, 0, 0, 0x40, 1, 0, 0, 0, 2, 0, 1, 2];
        assert_eq!(frame.encode(), expected);
    }

    #[test]
    fn passes_shared_test_vectors() {
        let unhex = |s: &str| -> Vec<u8> {
            (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap()).collect()
        };
        let vectors = include_str!("../../protocol/test_vectors.txt");
        let mut checked = 0;
        for line in vectors.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let f: Vec<&str> = line.split_whitespace().collect();
            let frame = Frame {
                flags: f[0].parse().unwrap(),
                pin_session_id: f[1].parse().unwrap(),
                sequence: f[2].parse().unwrap(),
                sample_counter: f[3].parse().unwrap(),
                codec: match f[4] {
                    "0" => Codec::RawPcm16,
                    "1" => Codec::Opus,
                    _ => Codec::ImaAdpcm,
                },
                payload: unhex(f[5]),
            };
            let expected = unhex(&f[6..].concat());
            assert_eq!(frame.encode(), expected);
            assert_eq!(Frame::decode(&expected).unwrap(), frame);
            checked += 1;
        }
        assert_eq!(checked, 2);
    }

    #[test]
    fn rejects_malformed_input_without_panicking() {
        let good = sample().encode();
        for len in 0..good.len() {
            assert_eq!(Frame::decode(&good[..len]), Err(FrameError::Truncated));
        }
        let mut bad = good.clone();
        bad[0] = 9;
        assert_eq!(Frame::decode(&bad), Err(FrameError::Version(9)));
        let mut bad = good;
        bad[12] = 9;
        assert_eq!(Frame::decode(&bad), Err(FrameError::Codec(9)));
    }
}
