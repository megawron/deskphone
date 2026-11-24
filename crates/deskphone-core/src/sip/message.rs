use std::collections::HashMap;
use anyhow::{anyhow, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SipMessageType {
    Request {
        method: String,
        uri: String,
    },
    Response {
        code: u16,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct SipMessage {
    pub msg_type: SipMessageType,
    pub version: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl SipMessage {
    pub fn new_request(method: &str, uri: &str) -> Self {
        Self {
            msg_type: SipMessageType::Request {
                method: method.to_string(),
                uri: uri.to_string(),
            },
            version: "SIP/2.0".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    pub fn new_response(code: u16, reason: &str) -> Self {
        Self {
            msg_type: SipMessageType::Response {
                code,
                reason: reason.to_string(),
            },
            version: "SIP/2.0".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    pub fn set_header(&mut self, name: &str, value: &str) {
        self.headers.insert(name.to_lowercase(), value.to_string());
    }

    pub fn get_header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_lowercase()).map(|s| s.as_str())
    }

    pub fn method(&self) -> Option<&str> {
        match &self.msg_type {
            SipMessageType::Request { method, .. } => Some(method),
            _ => None,
        }
    }

    pub fn status_code(&self) -> Option<u16> {
        match &self.msg_type {
            SipMessageType::Response { code, .. } => Some(*code),
            _ => None,
        }
    }

    pub fn cseq(&self) -> Option<(u32, String)> {
        let val = self.get_header("CSeq")?;
        let mut parts = val.split_whitespace();
        let num = parts.next()?.parse::<u32>().ok()?;
        let method = parts.next()?.to_string();
        Some((num, method))
    }

    pub fn call_id(&self) -> Option<&str> {
        self.get_header("Call-ID")
    }

    pub fn from_tag(&self) -> Option<String> {
        let val = self.get_header("From")?;
        extract_tag(val)
    }

    pub fn to_tag(&self) -> Option<String> {
        let val = self.get_header("To")?;
        extract_tag(val)
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = String::new();

        match &self.msg_type {
            SipMessageType::Request { method, uri } => {
                out.push_str(&format!("{} {} {}\r\n", method, uri, self.version));
            }
            SipMessageType::Response { code, reason } => {
                out.push_str(&format!("{} {} {}\r\n", self.version, code, reason));
            }
        }

        // Deterministische, RFC-übliche Header-Reihenfolge (Via zuerst)
        const ORDER: [&str; 8] = [
            "via",
            "route",
            "record-route",
            "max-forwards",
            "from",
            "to",
            "call-id",
            "cseq",
        ];
        for k in ORDER {
            if let Some(v) = self.headers.get(k) {
                out.push_str(&format!("{}: {}\r\n", title_case(k), v));
            }
        }
        let mut rest: Vec<(&String, &String)> = self
            .headers
            .iter()
            .filter(|(k, _)| !ORDER.contains(&k.as_str()) && k.as_str() != "content-length")
            .collect();
        rest.sort_by(|a, b| a.0.cmp(b.0));
        for (k, v) in rest {
            out.push_str(&format!("{}: {}\r\n", title_case(k), v));
        }

        out.push_str(&format!("Content-Length: {}\r\n\r\n", self.body.len()));

        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }

    pub fn parse(raw: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(raw)
            .map_err(|e| anyhow!("Invalid UTF-8 in SIP message: {}", e))?;

        let header_end = text
            .find("\r\n\r\n")
            .ok_or_else(|| anyhow!("Incomplete SIP header"))?;

        let header_section = &text[..header_end];
        let body_section = &raw[header_end + 4..];

        let mut lines = header_section.split("\r\n");
        let start_line = lines.next().ok_or_else(|| anyhow!("Empty SIP message"))?;

        let mut parts = start_line.split_whitespace();
        let first = parts.next().ok_or_else(|| anyhow!("Malformed start line"))?;

        let (msg_type, version) = if first.starts_with("SIP/") {
            // Response
            let code_str = parts.next().ok_or_else(|| anyhow!("Missing status code"))?;
            let code = code_str.parse::<u16>()?;
            let reason = parts.collect::<Vec<_>>().join(" ");
            (
                SipMessageType::Response { code, reason },
                first.to_string(),
            )
        } else {
            // Request
            let uri = parts.next().ok_or_else(|| anyhow!("Missing Request-URI"))?;
            let ver = parts.next().unwrap_or("SIP/2.0");
            (
                SipMessageType::Request {
                    method: first.to_string(),
                    uri: uri.to_string(),
                },
                ver.to_string(),
            )
        };

        let mut headers: HashMap<String, String> = HashMap::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            if let Some(colon_pos) = line.find(':') {
                let name = expand_compact_header(&line[..colon_pos].trim().to_lowercase());
                let val = line[colon_pos + 1..].trim().to_string();
                match headers.get_mut(&name) {
                    // RFC 3261 §7.3.1: Mehrfach-Header mit Listenwerten dürfen kommasepariert
                    // zusammengefasst werden. Für Via/Route/Record-Route ist das zwingend nötig,
                    // damit Antworten ALLE Via-Header zurückspiegeln.
                    Some(existing) if is_list_header(&name) => {
                        existing.push_str(", ");
                        existing.push_str(&val);
                    }
                    // Sonst gewinnt der erste Header (z. B. mehrere WWW-Authenticate)
                    Some(_) => {}
                    None => {
                        headers.insert(name, val);
                    }
                }
            }
        }

        // Body anhand Content-Length begrenzen (UDP-Pakete können Padding enthalten)
        let mut body = body_section.to_vec();
        if let Some(len) = headers
            .get("content-length")
            .and_then(|v| v.trim().parse::<usize>().ok())
        {
            if len < body.len() {
                body.truncate(len);
            }
        }

        Ok(Self {
            msg_type,
            version,
            headers,
            body,
        })
    }

    /// Returns the URI inside the `Contact` header (without angle brackets / parameters)
    pub fn contact_uri(&self) -> Option<String> {
        let val = self.get_header("Contact")?;
        extract_uri(val)
    }
}

/// Extracts the bare SIP URI from a name-addr / addr-spec header value
/// (e.g. `"Bob" <sip:101@pbx;transport=udp>;tag=1` -> `sip:101@pbx;transport=udp`).
pub fn extract_uri(header_val: &str) -> Option<String> {
    let v = header_val.trim();
    if let Some(start) = v.find('<') {
        let rest = &v[start + 1..];
        let end = rest.find('>')?;
        let uri = rest[..end].trim();
        if uri.is_empty() {
            None
        } else {
            Some(uri.to_string())
        }
    } else {
        let uri = v.split(';').next().unwrap_or("").trim();
        if uri.is_empty() {
            None
        } else {
            Some(uri.to_string())
        }
    }
}

/// Extracts the user part of a SIP URI inside a header (`"Bob" <sip:101@pbx>` -> `101`)
pub fn extract_user(header_val: &str) -> Option<String> {
    let uri = extract_uri(header_val)?;
    let after = uri
        .strip_prefix("sip:")
        .or_else(|| uri.strip_prefix("sips:"))
        .or_else(|| uri.strip_prefix("tel:"))
        .unwrap_or(&uri);
    let end = after
        .find(|c| c == '@' || c == ';' || c == '>' || c == '?')
        .unwrap_or(after.len());
    let user = &after[..end];
    if user.is_empty() {
        None
    } else {
        Some(user.to_string())
    }
}

fn is_list_header(name: &str) -> bool {
    matches!(name, "via" | "route" | "record-route" | "contact")
}

fn expand_compact_header(name: &str) -> String {
    match name {
        "v" => "via",
        "f" => "from",
        "t" => "to",
        "i" => "call-id",
        "m" => "contact",
        "l" => "content-length",
        "c" => "content-type",
        "k" => "supported",
        "s" => "subject",
        "e" => "content-encoding",
        "o" => "event",
        "r" => "refer-to",
        "b" => "referred-by",
        "x" => "session-expires",
        other => other,
    }
    .to_string()
}

fn title_case(k: &str) -> String {
    match k.to_lowercase().as_str() {
        "call-id" => "Call-ID".to_string(),
        "cseq" => "CSeq".to_string(),
        "www-authenticate" => "WWW-Authenticate".to_string(),
        "proxy-authenticate" => "Proxy-Authenticate".to_string(),
        "proxy-authorization" => "Proxy-Authorization".to_string(),
        "x-ippbxcalltype" => "X-IPPBXCallType".to_string(),
        "x-ippbxoriguserid" => "X-IPPBXOrigUserID".to_string(),
        "x-ippbxorigsiteid" => "X-IPPBXOrigSiteID".to_string(),
        "x-ippbxorigsiteguid" => "X-IPPBXOrigSiteGUID".to_string(),
        "x-ippbxorigsessionid" => "X-IPPBXOrigSessionID".to_string(),
        "x-ippbxorigip" => "X-IPPBXOrigIP".to_string(),
        "x-ippbxdestuserid" => "X-IPPBXDestUserID".to_string(),
        "x-ippbxdestsiteid" => "X-IPPBXDestSiteID".to_string(),
        "x-ippbxdestsiteguid" => "X-IPPBXDestSiteGUID".to_string(),
        "x-ippbxdestsessionid" => "X-IPPBXDestSessionID".to_string(),
        "x-ippbxdestip" => "X-IPPBXDestIP".to_string(),
        "x-ippbxcalledextension" => "X-IPPBXCalledExtension".to_string(),
        "x-ippbxcallednumber" => "X-IPPBXCalledNumber".to_string(),
        "x-ippbxcalledname" => "X-IPPBXCalledName".to_string(),
        "x-ippbxcalleduserid" => "X-IPPBXCalledUserID".to_string(),
        "x-ippbxcallinguserid" => "X-IPPBXCallingUserID".to_string(),
        "x-ippbxservercallid" => "X-IPPBXServerCallId".to_string(),
        other => other
            .split('-')
            .map(|seg| {
                let mut c = seg.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join("-"),
    }
}

fn extract_tag(header_val: &str) -> Option<String> {
    for part in header_val.split(';') {
        let trimmed = part.trim();
        if trimmed.starts_with("tag=") {
            return Some(trimmed["tag=".len()..].to_string());
        }
    }
    None
}
