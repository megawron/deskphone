use bytes::{Buf, BufMut, Bytes, BytesMut};
use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
pub struct RtpPacket {
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub marker: bool,
    pub payload: Bytes,
}

impl RtpPacket {
    pub fn new(payload_type: u8, seq: u16, timestamp: u32, ssrc: u32, payload: Bytes) -> Self {
        Self {
            payload_type,
            sequence_number: seq,
            timestamp,
            ssrc,
            marker: false,
            payload,
        }
    }

    pub fn serialize(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(12 + self.payload.len());
        // Byte 0: V=2, P=0, X=0, CC=0 -> 1000 0000 = 0x80
        buf.put_u8(0x80);
        // Byte 1: M (1 bit) | PT (7 bits)
        let pt = (if self.marker { 0x80 } else { 0x00 }) | (self.payload_type & 0x7F);
        buf.put_u8(pt);
        // Sequence number (2 bytes)
        buf.put_u16(self.sequence_number);
        // Timestamp (4 bytes)
        buf.put_u32(self.timestamp);
        // SSRC (4 bytes)
        buf.put_u32(self.ssrc);
        // Payload
        buf.put_slice(&self.payload);
        buf.freeze()
    }

    pub fn parse(mut raw: Bytes) -> Result<Self> {
        if raw.len() < 12 {
            return Err(anyhow!("RTP packet too short: {} bytes", raw.len()));
        }

        let b0 = raw.get_u8();
        let version = (b0 >> 6) & 0x03;
        if version != 2 {
            return Err(anyhow!("Unsupported RTP version: {}", version));
        }

        let b1 = raw.get_u8();
        let marker = (b1 & 0x80) != 0;
        let payload_type = b1 & 0x7F;

        let sequence_number = raw.get_u16();
        let timestamp = raw.get_u32();
        let ssrc = raw.get_u32();

        // CSRC-Liste, Header-Extension und Padding korrekt überspringen
        let csrc_count = (b0 & 0x0F) as usize;
        let has_extension = (b0 & 0x10) != 0;
        let has_padding = (b0 & 0x20) != 0;

        if raw.len() < csrc_count * 4 {
            return Err(anyhow!("RTP packet truncated in CSRC list"));
        }
        raw.advance(csrc_count * 4);

        if has_extension {
            if raw.len() < 4 {
                return Err(anyhow!("RTP packet truncated in header extension"));
            }
            let _profile = raw.get_u16();
            let ext_words = raw.get_u16() as usize;
            if raw.len() < ext_words * 4 {
                return Err(anyhow!("RTP header extension exceeds packet"));
            }
            raw.advance(ext_words * 4);
        }

        if has_padding && !raw.is_empty() {
            let pad = raw[raw.len() - 1] as usize;
            if pad <= raw.len() {
                raw.truncate(raw.len() - pad);
            }
        }

        let payload = raw;

        Ok(Self {
            payload_type,
            sequence_number,
            timestamp,
            ssrc,
            marker,
            payload,
        })
    }
}
