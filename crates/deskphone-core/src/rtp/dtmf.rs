use bytes::{BufMut, Bytes, BytesMut};

/// Represents an RFC 4733 / RFC 2833 telephone-event payload for DTMF signaling.
#[derive(Debug, Clone)]
pub struct DtmfEvent {
    pub event_code: u8,
    pub end_of_event: bool,
    pub volume: u8,
    pub duration: u16,
}

impl DtmfEvent {
    /// Converts a character ('0'-'9', '*', '#', 'A'-'D') to an RFC 4733 event code
    pub fn code_from_char(c: char) -> Option<u8> {
        match c {
            '0'..='9' => Some(c as u8 - b'0'),
            '*' => Some(10),
            '#' => Some(11),
            'a' | 'A' => Some(12),
            'b' | 'B' => Some(13),
            'c' | 'C' => Some(14),
            'd' | 'D' => Some(15),
            '!' => Some(16), // Flash
            _ => None,
        }
    }

    /// Serializes into the 4-byte RFC 4733 payload
    pub fn serialize(&self) -> Bytes {
        let mut buf = BytesMut::with_capacity(4);
        buf.put_u8(self.event_code);
        let second_byte = (if self.end_of_event { 0x80 } else { 0x00 }) | (self.volume & 0x3F);
        buf.put_u8(second_byte);
        buf.put_u16(self.duration);
        buf.freeze()
    }

    /// Generates a complete sequence of RFC 4733 packets for a single keypress (typically ~100ms)
    pub fn generate_keypress_sequence(digit: char) -> Option<Vec<Bytes>> {
        let code = Self::code_from_char(digit)?;
        let mut sequence = Vec::new();

        // Send 5 continuation frames (20ms each, duration incrementing: 160, 320, 480, 640, 800)
        for i in 1..=5 {
            let duration = (i * 160) as u16;
            let pkt = DtmfEvent {
                event_code: code,
                end_of_event: false,
                volume: 10,
                duration,
            };
            sequence.push(pkt.serialize());
        }

        // Send 3 final frames with End bit (E=1) set, as required by RFC 4733 Section 2.5.1.4
        for _ in 0..3 {
            let end_pkt = DtmfEvent {
                event_code: code,
                end_of_event: true,
                volume: 10,
                duration: 800,
            };
            sequence.push(end_pkt.serialize());
        }

        Some(sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dtmf_char_conversion() {
        assert_eq!(DtmfEvent::code_from_char('0'), Some(0));
        assert_eq!(DtmfEvent::code_from_char('5'), Some(5));
        assert_eq!(DtmfEvent::code_from_char('*'), Some(10));
        assert_eq!(DtmfEvent::code_from_char('#'), Some(11));
        assert_eq!(DtmfEvent::code_from_char('x'), None);
    }

    #[test]
    fn test_dtmf_serialization() {
        let event = DtmfEvent {
            event_code: 5,
            end_of_event: true,
            volume: 10,
            duration: 800,
        };
        let b = event.serialize();
        assert_eq!(b.len(), 4);
        assert_eq!(b[0], 5);
        assert_eq!(b[1], 0x80 | 10);
        assert_eq!(u16::from_be_bytes([b[2], b[3]]), 800);
    }
}
