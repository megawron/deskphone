use md5::{Digest, Md5};
use rand::Rng;

/// Computes SIP / HTTP Digest Authentication according to RFC 2617 & RFC 3261
pub struct DigestAuth;

impl DigestAuth {
    fn md5_hex(data: &str) -> String {
        let mut hasher = Md5::new();
        hasher.update(data.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Computes the Digest response string
    pub fn compute_response(
        username: &str,
        password: &str,
        realm: &str,
        nonce: &str,
        method: &str,
        uri: &str,
        qop: Option<&str>,
        nc: u32,
        cnonce: Option<&str>,
    ) -> String {
        let ha1 = Self::md5_hex(&format!("{}:{}:{}", username, realm, password));
        let ha2 = Self::md5_hex(&format!("{}:{}", method, uri));

        match qop {
            Some(q) if q == "auth" || q.contains("auth") => {
                let nc_str = format!("{:08x}", nc);
                let cn = cnonce.unwrap_or("0a4f113b");
                Self::md5_hex(&format!("{}:{}:{}:{}:{}:{}", ha1, nonce, nc_str, cn, "auth", ha2))
            }
            _ => Self::md5_hex(&format!("{}:{}:{}", ha1, nonce, ha2)),
        }
    }

    /// Parses a WWW-Authenticate or Proxy-Authenticate challenge header
    pub fn parse_challenge(header_val: &str) -> Option<(String, String, Option<String>)> {
        // Example: Digest realm="swyx.local", nonce="4f8a32b9", qop="auth", algorithm=MD5
        let realm = Self::extract_param(header_val, "realm")?;
        let nonce = Self::extract_param(header_val, "nonce")?;
        let qop = Self::extract_param(header_val, "qop");
        Some((realm, nonce, qop))
    }

    fn extract_param(header: &str, param_name: &str) -> Option<String> {
        let pattern = format!("{}=\"", param_name);
        if let Some(pos) = header.find(&pattern) {
            let start = pos + pattern.len();
            if let Some(end) = header[start..].find('"') {
                return Some(header[start..start + end].to_string());
            }
        }
        // Without quotes
        let pattern2 = format!("{}=", param_name);
        if let Some(pos) = header.find(&pattern2) {
            let start = pos + pattern2.len();
            let end = header[start..]
                .find(|c: char| c == ',' || c.is_whitespace())
                .unwrap_or(header.len() - start);
            return Some(header[start..start + end].to_string());
        }
        None
    }

    /// Formats an Authorization or Proxy-Authorization header
    pub fn format_authorization_header(
        is_proxy: bool,
        username: &str,
        realm: &str,
        nonce: &str,
        uri: &str,
        response: &str,
        qop: Option<&str>,
        nc: u32,
        cnonce: Option<&str>,
    ) -> (String, String) {
        let header_name = if is_proxy {
            "Proxy-Authorization".to_string()
        } else {
            "Authorization".to_string()
        };

        let mut val = format!(
            "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{}\", algorithm=MD5",
            username, realm, nonce, uri, response
        );

        if let Some(q) = qop {
            if q == "auth" || q.contains("auth") {
                let nc_str = format!("{:08x}", nc);
                let cn = cnonce.unwrap_or("0a4f113b");
                val.push_str(&format!(", qop=auth, nc={}, cnonce=\"{}\"", nc_str, cn));
            }
        }

        (header_name, val)
    }

    /// Builds a complete `Authorization` / `Proxy-Authorization` header for a 401/407 challenge,
    /// including `opaque` echoing (RFC 2617 §3.2.2), which many PBXs require.
    pub fn authorize(
        challenge: &str,
        is_proxy: bool,
        username: &str,
        password: &str,
        method: &str,
        uri: &str,
    ) -> Option<(String, String)> {
        let (realm, nonce, qop) = Self::parse_challenge(challenge)?;
        // qop kann eine Liste sein ("auth,auth-int") - wir unterstützen nur "auth"
        let qop = qop.filter(|q| q.split(',').any(|p| p.trim() == "auth"));
        let cnonce = Self::generate_cnonce();
        let response = Self::compute_response(
            username,
            password,
            &realm,
            &nonce,
            method,
            uri,
            qop.as_deref(),
            1,
            Some(&cnonce),
        );
        let (name, mut val) = Self::format_authorization_header(
            is_proxy,
            username,
            &realm,
            &nonce,
            uri,
            &response,
            qop.as_deref(),
            1,
            Some(&cnonce),
        );
        if let Some(opaque) = Self::extract_param(challenge, "opaque") {
            val.push_str(&format!(", opaque=\"{}\"", opaque));
        }
        Some((name, val))
    }

    /// Generates a random alphanumeric client nonce
    pub fn generate_cnonce() -> String {
        let mut rng = rand::thread_rng();
        let bytes: [u8; 8] = rng.gen();
        format!("{:x}", Md5::digest(bytes))
    }
}
