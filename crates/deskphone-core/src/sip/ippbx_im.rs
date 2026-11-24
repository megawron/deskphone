//! Swyx-eigenes Instant-Messaging-Format (`application/x-ippbxim+xml`).
//!
//! Der SwyxServer akzeptiert SIP `MESSAGE` nur mit einem der Content-Types
//! `application/x-ippbxcti+xml`, `application/x-ippbxim+xml` oder `application/x-ippbxgk`
//! (sonst `405 Method Not Allowed`). SwyxIt!/Swyx Messenger verwenden für Chat-Nachrichten
//! folgendes XML (rekonstruiert aus `CLMgr.exe`, `ChatCore.cpp`):
//!
//! ```xml
//! <IpPbxIM><Type>3</Type><Dir>1</Dir><ID>7</ID><TimeStamp>2026-10-14T16:00:00</TimeStamp><Text>Hallo</Text></IpPbxIM>
//! ```
//!
//! * `Type`: 0=None, 1=Hallo, 2=Bye, 3=Msg, 4=State
//! * `Dir`:  0=None, 1=Request, 2=ResponseOK, 3=ResponseTimeout
//!
//! Jede Anfrage (`Dir=1`) wird vom Empfänger mit derselben `Type`/`ID` und `Dir=2` quittiert.

pub const CONTENT_TYPE: &str = "application/x-ippbxim+xml; charset=utf-8";

pub const TYPE_HALLO: i32 = 1;
pub const TYPE_BYE: i32 = 2;
pub const TYPE_MSG: i32 = 3;
pub const TYPE_STATE: i32 = 4;

pub const DIR_REQUEST: i32 = 1;
pub const DIR_RESPONSE_OK: i32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpPbxIm {
    pub msg_type: i32,
    pub dir: i32,
    pub id: i32,
    pub timestamp: Option<String>,
    pub text: Option<String>,
}

impl IpPbxIm {
    pub fn new_message(id: i32, text: &str) -> Self {
        Self {
            msg_type: TYPE_MSG,
            dir: DIR_REQUEST,
            id,
            timestamp: Some(now_timestamp()),
            text: Some(text.to_string()),
        }
    }

    /// Quittung (`ResponseOK`) für eine empfangene Anfrage.
    pub fn response_ok(&self) -> Self {
        Self {
            msg_type: self.msg_type,
            dir: DIR_RESPONSE_OK,
            id: self.id,
            timestamp: Some(now_timestamp()),
            text: None,
        }
    }

    pub fn to_xml(&self) -> String {
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?><IpPbxIM>");
        xml.push_str(&format!("<Type>{}</Type>", self.msg_type));
        xml.push_str(&format!("<Dir>{}</Dir>", self.dir));
        xml.push_str(&format!("<ID>{}</ID>", self.id));
        if let Some(ts) = &self.timestamp {
            xml.push_str(&format!("<TimeStamp>{}</TimeStamp>", escape(ts)));
        }
        if let Some(text) = &self.text {
            xml.push_str(&format!("<Text>{}</Text>", escape(text)));
            let rtf_b64 = encode_swyx_rtf_base64(text);
            xml.push_str(&format!("<RTF>{}</RTF>", rtf_b64));
        }
        xml.push_str("</IpPbxIM>");
        xml
    }

    pub fn parse(xml: &str) -> Option<Self> {
        if !xml.contains("<IpPbxIM") {
            return None;
        }
        let num = |tag: &str| extract(xml, tag).and_then(|v| v.trim().parse::<i32>().ok());
        let plain_text = extract(xml, "Text")
            .map(|s| unescape(&s))
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                extract(xml, "RTF")
                    .map(|s| unescape(&s))
                    .and_then(|rtf_raw| decode_swyx_rtf_to_text(&rtf_raw))
                    .filter(|s| !s.trim().is_empty())
            });

        Some(Self {
            msg_type: num("Type").unwrap_or(0),
            dir: num("Dir").unwrap_or(0),
            id: num("ID").unwrap_or(0),
            timestamp: extract(xml, "TimeStamp").map(|s| unescape(&s)),
            text: plain_text,
        })
    }

    /// Echte Chat-Nachricht eines Kollegen (keine Quittung / kein Hallo/Bye)?
    pub fn is_incoming_text(&self) -> bool {
        self.msg_type == TYPE_MSG
            && self.dir == DIR_REQUEST
            && self.text.as_deref().map(|t| !t.trim().is_empty()).unwrap_or(false)
    }
}

pub fn is_ippbx_im_content_type(content_type: &str) -> bool {
    content_type.to_ascii_lowercase().contains("x-ippbxim+xml")
}

fn now_timestamp() -> String {
    // CLMgr.exe PTime::AsString(0) erwartet RFC 2822 / RFC 1123 ("wwwe, dd MMME yyyy hh:mm:ss z")
    chrono::Local::now().to_rfc2822()
}

/// Erzeugt den von SwyxIt! `IMClient.exe` (`0x417720` / `0x4178a0`) und `CLMgr.exe` (`0x62a406`)
/// erwarteten Base64-kodierten `<RTF>`-Block:
/// 5 Bytes ASCII-Smiley-Anzahl (`"00000"`) gefolgt vom Windows-RichEdit-RTF-Dokument.
pub fn encode_swyx_rtf_base64(text: &str) -> String {
    let mut rtf = String::from(
        "00000{\\rtf1\\ansi\\ansicpg1252\\deff0\\deflang1031{\\fonttbl{\\f0\\fnil\\fcharset0 Segoe UI;}}\\viewkind4\\uc1\\pard\\f0\\fs18 ",
    );
    for ch in text.chars() {
        match ch {
            '\\' => rtf.push_str("\\\\"),
            '{' => rtf.push_str("\\{"),
            '}' => rtf.push_str("\\}"),
            '\r' => {}
            '\n' => rtf.push_str("\\par\r\n"),
            c if (' '..='~').contains(&c) => rtf.push(c),
            c => {
                let mut buf = [0u16; 2];
                for &u in c.encode_utf16(&mut buf).iter() {
                    rtf.push_str(&format!("\\u{}?", u as i16));
                }
            }
        }
    }
    rtf.push_str("\\par\r\n}");
    base64_encode(rtf.as_bytes())
}

/// Dekodiert den Inhalt von `<RTF>...</RTF>` (Base64-kodiert mit `"00000"`-Präfix oder direktes RTF)
/// zu reinem UTF-8-Text, da SwyxIt! `IMClient.exe` (`0x4282a2`) ausschließlich `<RTF>` befüllt.
pub fn decode_swyx_rtf_to_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let rtf_str = if trimmed.starts_with("{\\rtf") {
        trimmed.to_string()
    } else if let Some(rest) = strip_swyx_smiley_header(trimmed) {
        if rest.starts_with("{\\rtf") {
            rest.to_string()
        } else {
            let decoded = base64_decode(trimmed)?;
            let decoded_str = latin1_or_utf8_to_string(&decoded);
            strip_swyx_smiley_header(&decoded_str)
                .unwrap_or(&decoded_str)
                .to_string()
        }
    } else {
        let decoded = base64_decode(trimmed)?;
        let decoded_str = latin1_or_utf8_to_string(&decoded);
        strip_swyx_smiley_header(&decoded_str)
            .unwrap_or(&decoded_str)
            .to_string()
    };

    if rtf_str.contains("{\\rtf") {
        let idx = rtf_str.find("{\\rtf").unwrap_or(0);
        Some(rtf_to_plain_text(&rtf_str[idx..]))
    } else {
        Some(rtf_str.trim_matches('\0').trim().to_string())
    }
}

fn strip_swyx_smiley_header(s: &str) -> Option<&str> {
    if s.len() >= 5 && s.as_bytes()[..5].iter().all(|b| b.is_ascii_digit()) {
        let count: usize = s[..5].parse().ok()?;
        let offset = 5usize.checked_add(count.checked_mul(10)?)?;
        if offset <= s.len() {
            return Some(&s[offset..]);
        }
    }
    None
}

fn latin1_or_utf8_to_string(bytes: &[u8]) -> String {
    let trimmed = match bytes.iter().rposition(|&b| b != 0) {
        Some(last) => &bytes[..=last],
        None => &[],
    };
    if let Ok(s) = std::str::from_utf8(trimmed) {
        s.to_string()
    } else {
        trimmed.iter().map(|&b| cp1252_to_char(b)).collect()
    }
}

fn rtf_to_plain_text(rtf: &str) -> String {
    let bytes = rtf.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut out = String::new();
    let mut depth: i32 = 0;
    let mut skip_depth: Option<i32> = None;
    let mut uc_skip: usize = 1;
    let mut pending_surrogate: Option<u16> = None;

    while i < len {
        let b = bytes[i];
        match b {
            b'{' => {
                depth += 1;
                i += 1;
                if skip_depth.is_none() {
                    let rest = &rtf[i..];
                    if rest.starts_with("\\*")
                        || rest.starts_with("\\fonttbl")
                        || rest.starts_with("\\colortbl")
                        || rest.starts_with("\\stylesheet")
                        || rest.starts_with("\\info")
                        || rest.starts_with("\\pict")
                        || rest.starts_with("\\object")
                    {
                        skip_depth = Some(depth);
                    }
                }
            }
            b'}' => {
                if skip_depth == Some(depth) {
                    skip_depth = None;
                }
                depth -= 1;
                i += 1;
                if depth <= 0 {
                    break;
                }
            }
            b'\\' => {
                i += 1;
                if i >= len {
                    break;
                }
                if skip_depth.is_some() {
                    let esc = bytes[i];
                    if esc == b'\\' || esc == b'{' || esc == b'}' {
                        i += 1;
                    } else if esc == b'\'' {
                        i = (i + 3).min(len);
                    } else if esc.is_ascii_alphabetic() {
                        while i < len && bytes[i].is_ascii_alphabetic() {
                            i += 1;
                        }
                        if i < len && (bytes[i] == b'-' || bytes[i].is_ascii_digit()) {
                            i += 1;
                            while i < len && bytes[i].is_ascii_digit() {
                                i += 1;
                            }
                        }
                        if i < len && bytes[i] == b' ' {
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                    continue;
                }

                let esc = bytes[i];
                match esc {
                    b'\\' | b'{' | b'}' => {
                        out.push(esc as char);
                        i += 1;
                    }
                    b'~' => {
                        out.push('\u{00A0}');
                        i += 1;
                    }
                    b'-' => {
                        i += 1;
                    }
                    b'_' => {
                        out.push('-');
                        i += 1;
                    }
                    b'\'' => {
                        i += 1;
                        if i + 2 <= len {
                            if let Ok(val) = u8::from_str_radix(&rtf[i..i + 2], 16) {
                                out.push(cp1252_to_char(val));
                            }
                            i += 2;
                        }
                    }
                    b'\r' | b'\n' => {
                        out.push('\n');
                        i += 1;
                    }
                    _ if esc.is_ascii_alphabetic() => {
                        let word_start = i;
                        while i < len && bytes[i].is_ascii_alphabetic() {
                            i += 1;
                        }
                        let word = &rtf[word_start..i];
                        let num_start = i;
                        if i < len && (bytes[i] == b'-' || bytes[i].is_ascii_digit()) {
                            i += 1;
                            while i < len && bytes[i].is_ascii_digit() {
                                i += 1;
                            }
                        }
                        let num_str = &rtf[num_start..i];
                        if i < len && bytes[i] == b' ' {
                            i += 1;
                        }

                        match word {
                            "par" | "line" => out.push('\n'),
                            "tab" => out.push('\t'),
                            "uc" => {
                                if let Ok(n) = num_str.parse::<usize>() {
                                    uc_skip = n;
                                }
                            }
                            "u" => {
                                if let Ok(n) = num_str.parse::<i32>() {
                                    let code_unit = (n as i16) as u16;
                                    if (0xD800..=0xDBFF).contains(&code_unit) {
                                        pending_surrogate = Some(code_unit);
                                    } else if (0xDC00..=0xDFFF).contains(&code_unit) {
                                        if let Some(high) = pending_surrogate.take() {
                                            if let Some(Ok(c)) =
                                                char::decode_utf16([high, code_unit]).next()
                                            {
                                                out.push(c);
                                            }
                                        }
                                    } else if let Some(c) = char::from_u32(code_unit as u32) {
                                        pending_surrogate = None;
                                        out.push(c);
                                    }
                                    // Skip uc_skip fallback chars (e.g. '?' or \'xx)
                                    let mut skipped = 0;
                                    while skipped < uc_skip && i < len {
                                        if bytes[i] == b'\\'
                                            && i + 1 < len
                                            && bytes[i + 1] == b'\''
                                        {
                                            i = (i + 4).min(len);
                                        } else if bytes[i] != b'{' && bytes[i] != b'}' {
                                            i += 1;
                                        } else {
                                            break;
                                        }
                                        skipped += 1;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            b'\r' | b'\n' => {
                i += 1;
            }
            _ => {
                if skip_depth.is_none() {
                    let ch = rtf[i..].chars().next().unwrap();
                    out.push(ch);
                    i += ch.len_utf8();
                } else {
                    i += 1;
                }
            }
        }
    }

    while out.ends_with('\n') || out.ends_with('\r') {
        out.pop();
    }
    out
}

fn cp1252_to_char(b: u8) -> char {
    match b {
        0x80 => '€',
        0x82 => '‚',
        0x83 => 'ƒ',
        0x84 => '„',
        0x85 => '…',
        0x86 => '†',
        0x87 => '‡',
        0x88 => 'ˆ',
        0x89 => '‰',
        0x8A => 'Š',
        0x8B => '‹',
        0x8C => 'Œ',
        0x8E => 'Ž',
        0x91 => '‘',
        0x92 => '’',
        0x93 => '“',
        0x94 => '”',
        0x95 => '•',
        0x96 => '–',
        0x97 => '—',
        0x98 => '˜',
        0x99 => '™',
        0x9A => 'š',
        0x9B => '›',
        0x9C => 'œ',
        0x9E => 'ž',
        0x9F => 'Ÿ',
        other => other as char,
    }
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;
    for b in input.bytes() {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            break;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

fn extract(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let inner = &xml[start..end];
    // CDATA-Blöcke unterstützen
    let inner = inner
        .strip_prefix("<![CDATA[")
        .and_then(|s| s.strip_suffix("]]>"))
        .unwrap_or(inner);
    Some(inner.to_string())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#13;", "\r")
        .replace("&#10;", "\n")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_message() {
        let im = IpPbxIm::new_message(5, "Hallo <Welt> & \"Kollegen\"");
        let xml = im.to_xml();
        assert!(xml.contains("<Type>3</Type><Dir>1</Dir><ID>5</ID>"));
        assert!(xml.contains("<RTF>"));
        let parsed = IpPbxIm::parse(&xml).unwrap();
        assert_eq!(parsed.text.as_deref(), Some("Hallo <Welt> & \"Kollegen\""));
        assert!(parsed.is_incoming_text());
    }

    #[test]
    fn roundtrip_rtf_only_like_swyxit_imclient() {
        // IMClient.exe (0x4282a2) sendet ausschließlich <RTF> (Base64 mit "00000" + RichEdit RTF) und KEIN <Text>!
        let rtf_b64 = encode_swyx_rtf_base64("Hallo E. Musterfrau, schöne Grüße!");
        let decoded_bytes = base64_decode(&rtf_b64).unwrap();
        assert!(decoded_bytes.starts_with(b"00000{\\rtf1"));

        let xml = format!(
            "<?xml version=\"1.0\"?><IpPbxIM><Type>3</Type><Dir>1</Dir><ID>1</ID><TimeStamp>Wed, 21 Oct 2026 08:30:00 +0200</TimeStamp><RTF>{}</RTF></IpPbxIM>",
            rtf_b64
        );
        let parsed = IpPbxIm::parse(&xml).unwrap();
        assert_eq!(
            parsed.text.as_deref(),
            Some("Hallo E. Musterfrau, schöne Grüße!")
        );
        assert!(parsed.is_incoming_text());
    }

    #[test]
    fn parses_cp1252_hex_escapes_in_swyxit_rtf() {
        let raw_swyx_rtf = b"00000{\\rtf1\\ansi\\ansicpg1252\\deff0\\deflang1031{\\fonttbl{\\f0\\fnil\\fcharset0 Segoe UI;}}\r\n\\viewkind4\\uc1\\pard\\f0\\fs18 Gr\\'fc\\'dfe & \\'e4\\'f6\\'fc\\par\r\n}";
        let b64 = base64_encode(raw_swyx_rtf);
        let xml = format!(
            "<IpPbxIM><Type>3</Type><Dir>1</Dir><ID>2</ID><RTF>{}</RTF></IpPbxIM>",
            b64
        );
        let parsed = IpPbxIm::parse(&xml).unwrap();
        assert_eq!(parsed.text.as_deref(), Some("Grüße & äöü"));
        assert!(parsed.is_incoming_text());
    }

    #[test]
    fn response_is_not_text() {
        let im = IpPbxIm::new_message(9, "x");
        let resp = IpPbxIm::parse(&im.response_ok().to_xml()).unwrap();
        assert_eq!(resp.dir, DIR_RESPONSE_OK);
        assert_eq!(resp.id, 9);
        assert!(!resp.is_incoming_text());
    }

    #[test]
    fn parses_swyx_payload_with_rtf() {
        let xml = "<IpPbxIM><Type>3</Type><Dir>1</Dir><ID>2</ID><TimeStamp>2026-10-14T12:00:00</TimeStamp><Text>hi</Text><RTF>{\\rtf1 hi}</RTF></IpPbxIM>";
        let im = IpPbxIm::parse(xml).unwrap();
        assert_eq!(im.text.as_deref(), Some("hi"));
        assert!(is_ippbx_im_content_type("application/x-ippbxim+xml; charset=utf-8"));
    }
}
