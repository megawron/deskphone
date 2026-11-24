use std::net::IpAddr;
use anyhow::{anyhow, Result};

/// RTP payload types the audio engine can actually encode/decode.
/// (G.711 A-law = 8, G.711 µ-law = 0). Opus is intentionally NOT offered,
/// because there is no Opus codec implementation in the RTP engine.
pub const SUPPORTED_AUDIO_PTS: [u8; 2] = [8, 0];

/// Default dynamic payload type for RFC 4733 telephone-events
pub const DEFAULT_DTMF_PT: u8 = 101;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDirection {
    SendRecv,
    SendOnly,
    RecvOnly,
    Inactive,
}

impl MediaDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            MediaDirection::SendRecv => "sendrecv",
            MediaDirection::SendOnly => "sendonly",
            MediaDirection::RecvOnly => "recvonly",
            MediaDirection::Inactive => "inactive",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().trim() {
            "sendonly" => MediaDirection::SendOnly,
            "recvonly" => MediaDirection::RecvOnly,
            "inactive" => MediaDirection::Inactive,
            _ => MediaDirection::SendRecv,
        }
    }

    /// Direction to use in an SDP answer for the given offered direction (RFC 3264 §6.1)
    pub fn answer_for(offer: MediaDirection) -> Self {
        match offer {
            MediaDirection::SendOnly => MediaDirection::RecvOnly,
            MediaDirection::RecvOnly => MediaDirection::SendOnly,
            MediaDirection::Inactive => MediaDirection::Inactive,
            MediaDirection::SendRecv => MediaDirection::SendRecv,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SdpSession {
    pub remote_ip: Option<IpAddr>,
    pub remote_port: Option<u16>,
    /// First mutually supported audio payload type (8 = PCMA, 0 = PCMU)
    pub selected_payload_type: u8,
    pub direction: MediaDirection,
    /// Negotiated RFC 4733 telephone-event payload type (if offered)
    pub dtmf_payload_type: Option<u8>,
    /// True if the remote side offered at least one codec we support
    pub has_supported_codec: bool,
}

impl SdpSession {
    /// Generates a standard SDP offer with G.711 A-law/µ-law and RFC 4733 DTMF
    pub fn build_offer(local_ip: &str, rtp_port: u16) -> String {
        Self::build_offer_with_direction(local_ip, rtp_port, MediaDirection::SendRecv)
    }

    /// Generates an SDP offer with custom media direction (e.g. sendonly for Hold, sendrecv for Unhold)
    pub fn build_offer_with_direction(local_ip: &str, rtp_port: u16, direction: MediaDirection) -> String {
        let sess_id = rand::random::<u32>() & 0x7fff_ffff;
        format!(
            "v=0\r\n\
            o=- {sess_id} {sess_id} IN IP4 {local_ip}\r\n\
            s=Deskphone\r\n\
            c=IN IP4 {local_ip}\r\n\
            t=0 0\r\n\
            m=audio {rtp_port} RTP/AVP 8 0 101\r\n\
            a=rtpmap:8 PCMA/8000\r\n\
            a=rtpmap:0 PCMU/8000\r\n\
            a=rtpmap:101 telephone-event/8000\r\n\
            a=fmtp:101 0-16\r\n\
            a=ptime:20\r\n\
            a={}\r\n",
            direction.as_str()
        )
    }

    /// Generates an SDP answer containing exactly the negotiated codec (+ optional DTMF)
    pub fn build_answer(
        local_ip: &str,
        rtp_port: u16,
        payload_type: u8,
        dtmf_payload_type: Option<u8>,
        direction: MediaDirection,
    ) -> String {
        let sess_id = rand::random::<u32>() & 0x7fff_ffff;
        let codec_name = if payload_type == 0 { "PCMU" } else { "PCMA" };
        let mut fmts = payload_type.to_string();
        let mut attrs = format!("a=rtpmap:{payload_type} {codec_name}/8000\r\n");
        if let Some(dtmf) = dtmf_payload_type {
            fmts.push_str(&format!(" {dtmf}"));
            attrs.push_str(&format!(
                "a=rtpmap:{dtmf} telephone-event/8000\r\na=fmtp:{dtmf} 0-16\r\n"
            ));
        }
        format!(
            "v=0\r\n\
            o=- {sess_id} {sess_id} IN IP4 {local_ip}\r\n\
            s=Deskphone\r\n\
            c=IN IP4 {local_ip}\r\n\
            t=0 0\r\n\
            m=audio {rtp_port} RTP/AVP {fmts}\r\n\
            {attrs}\
            a=ptime:20\r\n\
            a={}\r\n",
            direction.as_str()
        )
    }

    /// Parses an SDP offer or answer to extract remote IP, remote audio port, codec,
    /// DTMF payload type and stream direction
    pub fn parse_answer(sdp_text: &str) -> Result<Self> {
        let mut remote_ip = None;
        let mut remote_port = None;
        let mut offered_pts: Vec<u8> = Vec::new();
        let mut direction = MediaDirection::SendRecv;
        let mut dtmf_payload_type = None;
        let mut in_audio = false;
        let mut seen_audio = false;

        for line in sdp_text.lines() {
            let line = line.trim();
            if line.starts_with("m=") {
                // Nur die erste Audio-Zeile auswerten
                in_audio = line.starts_with("m=audio") && !seen_audio;
                if in_audio {
                    seen_audio = true;
                    // m=audio 10000 RTP/AVP 8 0 101
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(port) = parts[1].parse::<u16>() {
                            remote_port = Some(port);
                        }
                    }
                    offered_pts = parts
                        .iter()
                        .skip(3)
                        .filter_map(|p| p.parse::<u8>().ok())
                        .collect();
                }
            } else if line.starts_with("c=") {
                // c=IN IP4 192.168.1.1 (session-level oder media-level; media-level gewinnt)
                if !seen_audio || in_audio {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 3 {
                        if let Ok(ip) = parts[2].parse::<IpAddr>() {
                            if ip.is_unspecified() {
                                // c=IN IP4 0.0.0.0 indicates hold in RFC 2543
                                direction = MediaDirection::SendOnly;
                            }
                            remote_ip = Some(ip);
                        }
                    }
                }
            } else if let Some(attr) = line.strip_prefix("a=") {
                if !seen_audio || in_audio {
                    if attr == "sendonly" || attr == "recvonly" || attr == "inactive" || attr == "sendrecv" {
                        direction = MediaDirection::parse(attr);
                    } else if let Some(map) = attr.strip_prefix("rtpmap:") {
                        // a=rtpmap:101 telephone-event/8000
                        let mut it = map.split_whitespace();
                        if let (Some(pt), Some(enc)) = (it.next(), it.next()) {
                            if enc.to_lowercase().starts_with("telephone-event/8000") {
                                dtmf_payload_type = pt.parse::<u8>().ok();
                            }
                        }
                    }
                }
            }
        }

        let selected = offered_pts
            .iter()
            .copied()
            .find(|pt| SUPPORTED_AUDIO_PTS.contains(pt));
        if let Some(dtmf) = dtmf_payload_type {
            if !offered_pts.is_empty() && !offered_pts.contains(&dtmf) {
                dtmf_payload_type = None;
            }
        }

        let ip = remote_ip.ok_or_else(|| anyhow!("Could not extract remote IP from SDP"))?;
        let port = remote_port.ok_or_else(|| anyhow!("Could not extract remote audio port from SDP"))?;

        Ok(Self {
            remote_ip: Some(ip),
            remote_port: Some(port),
            selected_payload_type: selected.unwrap_or(8),
            direction,
            dtmf_payload_type,
            has_supported_codec: selected.is_some() || offered_pts.is_empty(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_contains_only_implemented_codecs() {
        let offer = SdpSession::build_offer("10.0.0.5", 10002);
        assert!(offer.contains("m=audio 10002 RTP/AVP 8 0 101"));
        assert!(!offer.contains("opus"));
    }

    #[test]
    fn parses_offer_with_opus_first_and_picks_g711() {
        let sdp = "v=0\r\no=- 1 1 IN IP4 10.0.0.10\r\ns=-\r\nc=IN IP4 10.0.0.10\r\nt=0 0\r\n\
                   m=audio 40000 RTP/AVP 111 0 8 96\r\na=rtpmap:111 opus/48000/2\r\n\
                   a=rtpmap:96 telephone-event/8000\r\na=sendrecv\r\n";
        let s = SdpSession::parse_answer(sdp).unwrap();
        assert_eq!(s.selected_payload_type, 0);
        assert_eq!(s.dtmf_payload_type, Some(96));
        assert!(s.has_supported_codec);
        assert_eq!(s.remote_port, Some(40000));
    }

    #[test]
    fn detects_unsupported_codecs() {
        let sdp = "v=0\r\nc=IN IP4 1.2.3.4\r\nm=audio 4000 RTP/AVP 9 18\r\n";
        let s = SdpSession::parse_answer(sdp).unwrap();
        assert!(!s.has_supported_codec);
    }

    #[test]
    fn answer_contains_single_codec() {
        let ans = SdpSession::build_answer("10.0.0.5", 10004, 0, Some(96), MediaDirection::SendRecv);
        assert!(ans.contains("m=audio 10004 RTP/AVP 0 96"));
        assert!(ans.contains("a=rtpmap:0 PCMU/8000"));
        assert!(ans.contains("a=rtpmap:96 telephone-event/8000"));
    }
}
