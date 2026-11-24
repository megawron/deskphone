//! Passive SIP registration diagnostic (no calls are ever placed).
//!
//! 1. Logs in to the SwyxServer (REST, fallback SOAP) with the credentials from
//!    `~/.config/deskphone/data.json` - exactly like the GTK client does at startup.
//! 2. Sends a SIP REGISTER (incl. digest auth) and prints the PBX answers.
//! 3. Immediately removes the binding again (REGISTER with Expires: 0).
//!
//! Incoming requests (e.g. INVITE) are deliberately ignored - this tool never answers calls.
//!
//! Usage: cargo run -p deskphone-core --example sip_diag

use std::time::Duration;

use anyhow::{anyhow, Result};
use deskphone_core::cds::{CdsClient, SwyxRestClient};
use deskphone_core::sip::{DigestAuth, SipMessage};
use tokio::net::UdpSocket;

fn mask(s: &str) -> String {
    format!("<{} Zeichen>", s.chars().count())
}

#[tokio::main]
async fn main() -> Result<()> {
    let home = std::env::var("HOME")?;
    let data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{home}/.config/deskphone/data.json"))?)?;
    let server = data["server_ip"].as_str().unwrap_or_default().to_string();
    let user = data["user_name"].as_str().unwrap_or_default().to_string();
    let password = data["password"].as_str().unwrap_or_default().to_string();
    println!("Server={server} User={user} Passwort={}", mask(&password));

    // --- 1. CDS Login (identisch zu SwyxProvider::connect) ---
    let rest = SwyxRestClient::new(server.clone(), 9100);
    let (sip_user, sip_pass, realm) = match rest.login(&user, &password).await {
        Ok(auth) => {
            println!("REST-Login OK (UserID {})", auth.user_id);
            let sip_realm = rest.get_sip_realm().await.unwrap_or_else(|| server.clone());
            let numbers = rest.get_numbers(auth.user_id, &auth.access_token).await.unwrap_or_default();
            println!("Nummern: {:?}, SIP-Realm: {sip_realm}", numbers.iter().map(|n| n.internal_number.clone()).collect::<Vec<_>>());
            match rest.get_sip_register_token(auth.user_id, &auth.access_token).await {
                Ok(tok) => {
                    let psk_utf8 = String::from_utf8(tok.preshared_key.clone());
                    println!(
                        "SIP-Register-Token: id={} psk_len={} psk_is_utf8={}",
                        tok.sip_register_id,
                        tok.preshared_key.len(),
                        psk_utf8.is_ok()
                    );
                    let sip_user = if !tok.sip_register_id.is_empty() {
                        tok.sip_register_id.clone()
                    } else {
                        numbers.first().and_then(|n| n.internal_number.clone()).unwrap_or(user.clone())
                    };
                    (sip_user, psk_utf8.unwrap_or(password.clone()), sip_realm)
                }
                Err(e) => {
                    println!("SIP-Register-Token nicht verfügbar: {e}");
                    (user.clone(), password.clone(), sip_realm)
                }
            }
        }
        Err(e) => {
            println!("REST-Login fehlgeschlagen ({e}) -> SOAP");
            let cds = CdsClient::new(server.clone(), 8094, false);
            let auth = cds.login(&user, &password).await?;
            let creds = cds.get_sip_credentials(auth.user_id, &user, &auth.access_token).await?;
            println!(
                "SOAP OK: sip_user_name={} sip_user_id={} realm={} has_psk={}",
                creds.sip_user_name,
                creds.sip_user_id,
                creds.sip_realm,
                creds.psk.is_some()
            );
            let u = if !creds.sip_user_id.is_empty() { creds.sip_user_id } else { creds.sip_user_name };
            let r = if creds.sip_realm.is_empty() { server.clone() } else { creds.sip_realm };
            (u, creds.psk.unwrap_or(password.clone()), r)
        }
    };
    println!("SIP-User={sip_user} Realm={realm} SIP-Passwort={}", mask(&sip_pass));

    // --- 2. REGISTER ---
    let local_ip = deskphone_core::provider::network::get_local_ip_for(&server, 5060);
    let sock = UdpSocket::bind(format!("{local_ip}:0")).await?;
    let local_port = sock.local_addr()?.port();
    let server_addr = format!("{server}:5060");
    let uri_user = sip_user.replace('{', "%7b").replace('}', "%7d");
    let call_id = format!("{}@{}", uuid::Uuid::new_v4(), local_ip);
    let tag = format!("{:x}", rand::random::<u32>());

    let mut cseq = 1u32;
    let mut auth_hdr: Option<(String, String)> = None;
    for expires in [60u32, 0u32] {
        for attempt in 0..2 {
            let mut reg = SipMessage::new_request("REGISTER", &format!("sip:{realm}"));
            reg.set_header(
                "Via",
                &format!("SIP/2.0/UDP {local_ip}:{local_port};branch=z9hG4bK{};rport", uuid::Uuid::new_v4().simple()),
            );
            reg.set_header("Max-Forwards", "70");
            reg.set_header("From", &format!("<sip:{uri_user}@{realm}>;tag={tag}"));
            reg.set_header("To", &format!("<sip:{uri_user}@{realm}>"));
            reg.set_header("Call-ID", &call_id);
            reg.set_header("CSeq", &format!("{cseq} REGISTER"));
            reg.set_header("Contact", &format!("<sip:{uri_user}@{local_ip}:{local_port}>"));
            reg.set_header("Expires", &expires.to_string());
            reg.set_header("User-Agent", "Deskphone-Diag/0.1");
            if let Some((n, v)) = &auth_hdr {
                reg.set_header(n, v);
            }
            cseq += 1;
            sock.send_to(&reg.serialize(), &server_addr).await?;

            let resp = loop {
                let mut buf = vec![0u8; 65535];
                let (len, _) = tokio::time::timeout(Duration::from_secs(5), sock.recv_from(&mut buf))
                    .await
                    .map_err(|_| anyhow!("Timeout - keine Antwort vom SIP-Server auf Port 5060"))??;
                let Ok(m) = SipMessage::parse(&buf[..len]) else { continue };
                if let Some(method) = m.method() {
                    println!("  (ignoriere eingehende Anfrage {method})");
                    continue;
                }
                if m.call_id() != Some(call_id.as_str()) {
                    continue;
                }
                let code = m.status_code().unwrap_or(0);
                if code < 200 {
                    continue;
                }
                break m;
            };
            let code = resp.status_code().unwrap_or(0);
            println!("REGISTER (Expires {expires}, Versuch {attempt}) -> {code}");
            for h in ["www-authenticate", "proxy-authenticate", "contact", "expires", "server", "warning"] {
                if let Some(v) = resp.get_header(h) {
                    println!("    {h}: {v}");
                }
            }
            if code == 401 || code == 407 {
                let ch = resp
                    .get_header("www-authenticate")
                    .or_else(|| resp.get_header("proxy-authenticate"))
                    .ok_or_else(|| anyhow!("Challenge ohne Header"))?;
                let (r, nonce, qop) = DigestAuth::parse_challenge(ch).ok_or_else(|| anyhow!("Challenge unlesbar"))?;
                let uri = format!("sip:{realm}");
                let cn = DigestAuth::generate_cnonce();
                let digest =
                    DigestAuth::compute_response(&sip_user, &sip_pass, &r, &nonce, "REGISTER", &uri, qop.as_deref(), 1, Some(&cn));
                auth_hdr = Some(DigestAuth::format_authorization_header(
                    code == 407,
                    &sip_user,
                    &r,
                    &nonce,
                    &uri,
                    &digest,
                    qop.as_deref(),
                    1,
                    Some(&cn),
                ));
                continue;
            }
            break;
        }
    }
    println!("Fertig (Binding wieder entfernt).");
    Ok(())
}
